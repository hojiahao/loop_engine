use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loop_protocol::wire::jobs::v1::{PrepareJobArtifactsRequest, PrepareJobArtifactsResponse};
use loop_protocol::wire::v1::{Actor, JobRecord};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifests::files::ReadBudget;
use crate::manifests::{LocalArtifacts, ObjectRef, data};
use crate::store::{PgJobStore, StoreError, StoreResult};

/// Exact server-pinned data manifest for an existing job. Neither this reference
/// nor the returned view ID grants authority without current connection and lease.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataPin {
    /// Exact immutable durable job ID.
    pub job_id: String,
    /// Actual content-addressed data declaration, bound to the job's input.
    pub manifest: ObjectRef,
    /// Selects the isolated source namespace, never a caller-controlled URI.
    pub protected: bool,
}

/// Broker-owned namespaces and bounded, read-only data-view publication.
/// Workers must mount only the returned view, never the source/parent directory.
pub struct ArtifactBroker {
    development: Arc<LocalArtifacts>,
    protected: Arc<LocalArtifacts>,
    views: PathBuf,
    view_identity: (u64, u64),
    pins: BTreeMap<String, DataPin>,
    publications: tokio::sync::Semaphore,
}

impl ArtifactBroker {
    /// Open three distinct, non-nested, canonical deployment directories.
    /// The view parent must be owned by the runtime UID and mode 0700. Source
    /// directories must not be group/world writable; the protected source must
    /// also be runtime-owned and mode 0700. No directories are created
    /// or credentials accepted; invalid layout denies startup.
    pub fn open(
        development: &Path,
        protected: &Path,
        views: &Path,
        pins: Vec<DataPin>,
    ) -> StoreResult<Self> {
        let paths = [development, protected, views];
        for (index, path) in paths.iter().enumerate() {
            if !path.is_absolute()
                || std::fs::canonicalize(path).ok().as_deref() != Some(*path)
                || paths.iter().enumerate().any(|(other, candidate)| {
                    index != other && (path.starts_with(candidate) || candidate.starts_with(path))
                })
            {
                return Err(StoreError::Invalid("runtime data namespace"));
            }
            let metadata = std::fs::metadata(path)
                .map_err(|_| StoreError::Unavailable("runtime namespace"))?;
            if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
                return Err(StoreError::Invalid("writable runtime namespace"));
            }
        }
        let protected_metadata = std::fs::metadata(protected)
            .map_err(|_| StoreError::Unavailable("protected namespace"))?;
        if protected_metadata.mode() & 0o777 != 0o700
            || protected_metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(StoreError::Invalid("protected namespace ownership"));
        }
        let metadata =
            std::fs::metadata(views).map_err(|_| StoreError::Unavailable("runtime views"))?;
        if metadata.mode() & 0o777 != 0o700 || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(StoreError::Invalid("runtime view ownership"));
        }
        if pins.len() > 4096 {
            return Err(StoreError::Invalid("runtime data registry bounds"));
        }
        let mut entries = BTreeMap::new();
        for pin in pins {
            crate::store::validate_id(&pin.job_id)?;
            pin.manifest.digest()?;
            if entries.insert(pin.job_id.clone(), pin).is_some() {
                return Err(StoreError::Invalid("duplicate runtime data job"));
            }
        }
        Ok(Self {
            development: Arc::new(LocalArtifacts::open(development)?),
            protected: Arc::new(LocalArtifacts::open(protected)?),
            views: views.to_owned(),
            view_identity: (metadata.dev(), metadata.ino()),
            pins: entries,
            publications: tokio::sync::Semaphore::new(2),
        })
    }

    pub(crate) async fn prepare(
        &self,
        store: &PgJobStore,
        actor: &Actor,
        command: &PrepareJobArtifactsRequest,
        job: &JobRecord,
    ) -> StoreResult<PrepareJobArtifactsResponse> {
        let _permit = self
            .publications
            .try_acquire()
            .map_err(|_| StoreError::Unavailable("data publication capacity"))?;
        self.check_views()?;
        let job_id = &command
            .job_id
            .as_ref()
            .ok_or(StoreError::Invalid("data job ID"))?
            .value;
        let lease_id = &command
            .lease_id
            .as_ref()
            .ok_or(StoreError::Invalid("data lease ID"))?
            .value;
        let pin = self.pins.get(job_id).ok_or(StoreError::AdmissionDenied)?;
        let protected = super::service::protected(job);
        if protected != pin.protected {
            return Err(StoreError::AdmissionDenied);
        }
        let specification = job
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("data job specification"))?;
        let source = if protected {
            &self.protected
        } else {
            &self.development
        };
        let evidence = data::resolve(source, &pin.manifest, specification, protected).await?;
        let view_id = view_id(job_id, lease_id, &pin.manifest)?;
        let pending = tempfile::Builder::new()
            .prefix(".loop-view-")
            .tempdir_in(&self.views)
            .map_err(|_| StoreError::Unavailable("runtime staging directory"))?;
        evidence.copy_to(pending.path()).await?;
        let response = store.accept_data_access(actor, command, &evidence).await?;
        if response.view_id != view_id {
            return Err(StoreError::Corrupt("data view receipt"));
        }
        // A receipt accepts publication, not delivery. Recheck before exposing a
        // complete view, and preserve an existing immutable view on any conflict.
        store.runtime_lease(actor, job_id, lease_id).await?;
        evidence.check(specification)?;
        self.check_views()?;
        std::fs::set_permissions(pending.path(), std::fs::Permissions::from_mode(0o555))?;
        let target = self.views.join(&view_id);
        match renameat_with(CWD, pending.path(), CWD, &target, RenameFlags::NOREPLACE) {
            Ok(()) => {
                File::open(&self.views)?.sync_all()?;
            }
            Err(rustix::io::Errno::EXIST) => {
                // Restore owner write permission so TempDir can remove only this
                // request's unpublished temporary copy when the operation ends.
                std::fs::set_permissions(pending.path(), std::fs::Permissions::from_mode(0o700))?;
                self.verify_view(&target, &evidence.artifacts()).await?;
            }
            Err(_) => {
                std::fs::set_permissions(pending.path(), std::fs::Permissions::from_mode(0o700))?;
                return Err(StoreError::Unavailable("runtime view publication"));
            }
        }
        Ok(response)
    }

    fn check_views(&self) -> StoreResult<()> {
        let metadata = std::fs::symlink_metadata(&self.views)
            .map_err(|_| StoreError::Unavailable("runtime views"))?;
        if !metadata.is_dir()
            || (metadata.dev(), metadata.ino()) != self.view_identity
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(StoreError::Corrupt("runtime view directory replaced"));
        }
        Ok(())
    }

    async fn verify_view(
        &self,
        target: &Path,
        artifacts: &[loop_protocol::wire::v1::ArtifactRef],
    ) -> StoreResult<()> {
        let source = LocalArtifacts::open(target)?;
        let count = std::fs::read_dir(target)?.take(129).count();
        if count != artifacts.len() {
            return Err(StoreError::Corrupt("runtime view membership"));
        }
        let mut budget = ReadBudget::new();
        for artifact in artifacts {
            let reference = ObjectRef {
                sha256: artifact
                    .artifact_id
                    .as_ref()
                    .ok_or(StoreError::Corrupt("runtime view identity"))?
                    .value
                    .clone(),
                byte_size: artifact.byte_size,
            };
            source.load(&reference, false, &mut budget).await?.check()?;
        }
        Ok(())
    }
}

pub(crate) fn view_id(job_id: &str, lease_id: &str, manifest: &ObjectRef) -> StoreResult<String> {
    let mut hash = Sha256::new();
    hash.update(b"loop.data-view/v1\0");
    hash.update(
        serde_json::to_vec(&[job_id, lease_id, &manifest.sha256])
            .map_err(|_| StoreError::Invalid("runtime view identity"))?,
    );
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
