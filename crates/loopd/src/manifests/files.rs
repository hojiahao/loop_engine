use std::collections::BTreeMap;
use std::fs::{File, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use loop_core::factor::ExpressionId;
use rustix::fs::{Mode, OFlags, open, openat};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::store::{StoreError, StoreResult};

const METADATA_LIMIT: u64 = 1_048_576;
const OBJECT_LIMIT: usize = 16_384;
const BYTE_LIMIT: u64 = 64 * 1024 * 1024 * 1024;
const OPEN_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);

/// Byte identity, not an authority-bearing URI or a semantic factor identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectRef {
    /// Lowercase `sha256:` followed by exactly 64 hexadecimal digits.
    pub sha256: String,
    /// Exact file size; objects are never read past this bound.
    pub byte_size: u64,
}

impl ObjectRef {
    pub(crate) fn digest(&self) -> StoreResult<[u8; 32]> {
        ExpressionId::parse(&self.sha256)
            .map(|id| *id.as_bytes())
            .map_err(|_| StoreError::Invalid("artifact digest"))
    }

    fn name(&self) -> StoreResult<&str> {
        self.digest()?;
        Ok(&self.sha256[7..])
    }
}

/// Read-only, flat content-addressed store under a deployment-selected root.
/// Opening the store grants no research or holdout authority. No caller URI is
/// followed. Each object is checksummed outside transactions, then version-
/// checked at consumption. The cache is bounded and never hides changed files.
pub struct LocalArtifacts {
    root: Arc<File>,
    cache: Mutex<BTreeMap<String, Arc<VerifiedFile>>>,
}

impl LocalArtifacts {
    /// Pin an absolute, existing directory without following its final symlink.
    /// Parent directories are deployment configuration, not request input.
    /// Errors do not disclose the configured path or create any files.
    pub fn open(root: &Path) -> StoreResult<Self> {
        if !root.is_absolute() {
            return Err(StoreError::Invalid("artifact root"));
        }
        let fd = open(root, OPEN_FLAGS | OFlags::DIRECTORY, Mode::empty())
            .map_err(|_| StoreError::Unavailable("artifact root"))?;
        Ok(Self {
            root: Arc::new(File::from(fd)),
            cache: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) async fn load(
        &self,
        reference: &ObjectRef,
        metadata: bool,
        budget: &mut ReadBudget,
    ) -> StoreResult<Arc<VerifiedFile>> {
        budget.add(reference.byte_size, metadata)?;
        if metadata && reference.byte_size > METADATA_LIMIT {
            return Err(StoreError::Invalid("manifest size"));
        }
        let cached = self
            .cache
            .lock()
            .map_err(|_| StoreError::Unavailable("artifact cache"))?
            .get(&reference.sha256)
            .cloned();
        if let Some(cached) = cached
            && cached.reference == *reference
            && (!metadata || cached.bytes.is_some())
            && cached.check().is_ok()
        {
            return Ok(cached);
        }
        let name = reference.name()?.to_owned();
        let file = open_leaf(&self.root, &name)?;
        let before = version(&file)?;
        if before.size != reference.byte_size {
            return Err(StoreError::Corrupt("artifact size"));
        }
        let reader = file
            .try_clone()
            .map_err(|_| StoreError::Unavailable("artifact descriptor"))?;
        let remaining = budget.remaining()?;
        let read = async {
            let mut reader = tokio::fs::File::from_std(reader).take(reference.byte_size + 1);
            let mut hash = Sha256::new();
            let mut retained = metadata.then(Vec::new);
            let mut buffer = vec![0_u8; 65_536];
            let mut count = 0_u64;
            loop {
                budget.check_time()?;
                let length = reader
                    .read(&mut buffer)
                    .await
                    .map_err(|_| StoreError::Unavailable("artifact read"))?;
                if length == 0 {
                    break;
                }
                count += length as u64;
                hash.update(&buffer[..length]);
                if let Some(bytes) = &mut retained {
                    bytes.extend_from_slice(&buffer[..length]);
                }
            }
            if count != reference.byte_size || hash.finalize().as_slice() != reference.digest()? {
                return Err(StoreError::Corrupt("artifact checksum"));
            }
            Ok(retained.map(Arc::<[u8]>::from))
        };
        let bytes = tokio::time::timeout(remaining, read)
            .await
            .map_err(|_| StoreError::Unavailable("artifact verification timeout"))??;
        let verified = Arc::new(VerifiedFile {
            root: Arc::clone(&self.root),
            file,
            name,
            version: before,
            reference: reference.clone(),
            bytes,
        });
        verified.check()?;
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| StoreError::Unavailable("artifact cache"))?;
        let metadata_bytes = verified.bytes.as_ref().map_or(0, |bytes| bytes.len());
        let over_memory = metadata_bytes > 0
            && cache
                .values()
                .filter_map(|file| file.bytes.as_ref())
                .map(|bytes| bytes.len())
                .sum::<usize>()
                + metadata_bytes
                > 32 * METADATA_LIMIT as usize;
        if over_memory || (cache.len() >= OBJECT_LIMIT && !cache.contains_key(&reference.sha256)) {
            // Eviction affects performance only. Active verifications retain
            // their own descriptors, so clearing cannot invalidate their proof.
            cache.clear();
        }
        cache.insert(reference.sha256.clone(), Arc::clone(&verified));
        Ok(verified)
    }
}

pub(crate) struct ReadBudget {
    objects: usize,
    bytes: u64,
    metadata_bytes: u64,
    started: Instant,
}

impl ReadBudget {
    pub(crate) fn new() -> Self {
        Self {
            objects: 0,
            bytes: 0,
            metadata_bytes: 0,
            started: Instant::now(),
        }
    }

    fn add(&mut self, size: u64, metadata: bool) -> StoreResult<()> {
        self.objects += 1;
        self.bytes = self
            .bytes
            .checked_add(size)
            .ok_or(StoreError::Invalid("artifact byte budget"))?;
        if self.objects > OBJECT_LIMIT || self.bytes > BYTE_LIMIT {
            return Err(StoreError::Invalid("artifact verification budget"));
        }
        if metadata {
            self.metadata_bytes = self
                .metadata_bytes
                .checked_add(size)
                .ok_or(StoreError::Invalid("manifest memory budget"))?;
            if self.metadata_bytes > 16 * METADATA_LIMIT {
                return Err(StoreError::Invalid("manifest memory budget"));
            }
        }
        self.check_time()
    }

    fn remaining(&self) -> StoreResult<Duration> {
        Duration::from_secs(10)
            .checked_sub(self.started.elapsed())
            .filter(|duration| !duration.is_zero())
            .ok_or(StoreError::Unavailable("artifact verification timeout"))
    }

    fn check_time(&self) -> StoreResult<()> {
        self.remaining().map(|_| ())
    }
}

pub(crate) struct VerifiedFile {
    root: Arc<File>,
    file: File,
    name: String,
    version: FileVersion,
    reference: ObjectRef,
    bytes: Option<Arc<[u8]>>,
}

impl VerifiedFile {
    pub(crate) fn check(&self) -> StoreResult<()> {
        if version(&self.file)? != self.version
            || version(&open_leaf(&self.root, &self.name)?)? != self.version
        {
            return Err(StoreError::Corrupt("artifact changed after verification"));
        }
        Ok(())
    }

    pub(crate) fn bytes(&self) -> StoreResult<&[u8]> {
        self.check()?;
        self.bytes
            .as_deref()
            .ok_or(StoreError::Invalid("non-metadata artifact"))
    }

    pub(crate) fn json<T: DeserializeOwned + Serialize>(&self) -> StoreResult<T> {
        let bytes = self.bytes()?;
        let parsed: T =
            serde_json::from_slice(bytes).map_err(|_| StoreError::Corrupt("manifest JSON"))?;
        if serde_json::to_vec(&parsed).map_err(|_| StoreError::Corrupt("manifest encoding"))?
            != bytes
        {
            return Err(StoreError::Corrupt("non-canonical manifest"));
        }
        Ok(parsed)
    }

    pub(crate) async fn copy_to(&self, path: &Path) -> StoreResult<()> {
        self.check()?;
        // A fresh open-file description avoids sharing a cursor between copies.
        let input = open_leaf(&self.root, &self.name)?;
        if version(&input)? != self.version {
            return Err(StoreError::Corrupt("artifact copy version"));
        }
        let mut input = tokio::fs::File::from_std(input).take(self.reference.byte_size + 1);
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .await
            .map_err(|_| StoreError::Unavailable("artifact view output"))?;
        let mut hash = Sha256::new();
        let mut count = 0_u64;
        let mut buffer = [0_u8; 65_536];
        loop {
            let length = input
                .read(&mut buffer)
                .await
                .map_err(|_| StoreError::Unavailable("artifact copy read"))?;
            if length == 0 {
                break;
            }
            count += length as u64;
            hash.update(&buffer[..length]);
            output
                .write_all(&buffer[..length])
                .await
                .map_err(|_| StoreError::Unavailable("artifact copy write"))?;
        }
        if count != self.reference.byte_size
            || hash.finalize().as_slice() != self.reference.digest()?
        {
            return Err(StoreError::Corrupt("artifact copy checksum"));
        }
        self.check()?;
        output
            .sync_all()
            .await
            .map_err(|_| StoreError::Unavailable("artifact view sync"))?;
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileVersion {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl From<Metadata> for FileVersion {
    fn from(value: Metadata) -> Self {
        Self {
            device: value.dev(),
            inode: value.ino(),
            size: value.len(),
            modified: (value.mtime(), value.mtime_nsec()),
            changed: (value.ctime(), value.ctime_nsec()),
        }
    }
}

fn version(file: &File) -> StoreResult<FileVersion> {
    let metadata = file
        .metadata()
        .map_err(|_| StoreError::Unavailable("artifact metadata"))?;
    if !metadata.is_file() {
        return Err(StoreError::Invalid("artifact must be a regular file"));
    }
    Ok(metadata.into())
}

fn open_leaf(root: &File, name: &str) -> StoreResult<File> {
    openat(root, name, OPEN_FLAGS, Mode::empty())
        .map(File::from)
        .map_err(|_| StoreError::Unavailable("artifact object"))
}
