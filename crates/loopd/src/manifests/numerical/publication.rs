use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

impl NumericalResolver {
    pub(in crate::manifests) fn check_root(&self) -> StoreResult<()> {
        let metadata = std::fs::symlink_metadata(&self.root)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o022 != 0
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || (metadata.dev(), metadata.ino()) != self.identity
        {
            return Err(StoreError::Corrupt("numerical publication root changed"));
        }
        Ok(())
    }

    pub(in crate::manifests) async fn json<T: Serialize>(
        &self,
        value: &T,
        replay: bool,
        files: &mut Vec<Arc<VerifiedFile>>,
    ) -> StoreResult<ObjectRef> {
        let bytes = serde_json::to_vec(value)
            .map_err(|_| StoreError::Corrupt("numerical manifest encoding"))?;
        self.publish(&bytes, replay, files).await
    }

    pub(in crate::manifests) async fn publish(
        &self,
        bytes: &[u8],
        replay: bool,
        files: &mut Vec<Arc<VerifiedFile>>,
    ) -> StoreResult<ObjectRef> {
        self.check_root()?;
        if bytes.is_empty() || bytes.len() > 1_048_576 {
            return Err(StoreError::Invalid("numerical manifest bound"));
        }
        let reference = ObjectRef {
            sha256: format!("sha256:{:x}", Sha256::digest(bytes)),
            byte_size: bytes.len() as u64,
        };
        if !replay {
            let root = self.root.clone();
            let name = reference.sha256[7..].to_owned();
            let bytes = bytes.to_vec();
            tokio::task::spawn_blocking(move || -> StoreResult<()> {
                let mut temporary = tempfile::Builder::new()
                    .prefix(".loop-numerical-")
                    .tempfile_in(&root)?;
                temporary.write_all(&bytes)?;
                temporary.as_file().sync_all()?;
                match temporary.persist_noclobber(root.join(name)) {
                    Ok(_) => {}
                    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(StoreError::Io(error.error)),
                }
                std::fs::File::open(root)?.sync_all()?;
                Ok(())
            })
            .await
            .map_err(|_| StoreError::Unavailable("numerical publication"))??;
        }
        self.check_root()?;
        files.push(
            self.artifacts
                .load(&reference, true, &mut ReadBudget::new())
                .await?,
        );
        Ok(reference)
    }

    pub(super) async fn copy_result(
        &self,
        source: &LocalArtifacts,
        artifact: &wire::ArtifactRef,
        replay: bool,
        files: &mut Vec<Arc<VerifiedFile>>,
    ) -> StoreResult<ObjectRef> {
        self.check_root()?;
        let reference = ObjectRef {
            sha256: artifact
                .artifact_id
                .as_ref()
                .ok_or(StoreError::Corrupt("numerical output ID"))?
                .value
                .clone(),
            byte_size: artifact.byte_size,
        };
        let file = source
            .load(&reference, false, &mut ReadBudget::new())
            .await?;
        if !replay {
            let directory = tempfile::Builder::new()
                .prefix(".loop-numerical-")
                .tempdir_in(&self.root)?;
            let temporary = directory.path().join("object");
            file.copy_to(&temporary).await?;
            self.check_root()?;
            match std::fs::hard_link(&temporary, self.root.join(&reference.sha256[7..])) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(StoreError::Io(error)),
            }
            std::fs::File::open(&self.root)?.sync_all()?;
        }
        files.push(file);
        files.push(
            self.artifacts
                .load(&reference, false, &mut ReadBudget::new())
                .await?,
        );
        self.check_root()?;
        Ok(reference)
    }
}
