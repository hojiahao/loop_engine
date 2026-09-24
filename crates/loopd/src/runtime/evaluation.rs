use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use loop_protocol::wire::v1::{FactorEvaluationResult, FactorEvaluationWork};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::manifests::{EvaluationResolver, LocalArtifacts};
use crate::store::{StoreError, StoreResult};

/// Fixed installed Python evaluator with a private immutable output namespace.
/// This is trusted service code, not a generic Agent command or OS sandbox.
/// No inherited provider/database environment reaches the numerical process.
pub struct FactorExecutor {
    pub(super) resolver: Arc<EvaluationResolver>,
    pub(super) outputs: LocalArtifacts,
    python: PathBuf,
    output: PathBuf,
    output_identity: (u64, u64),
    pub(super) permits: tokio::sync::Semaphore,
}

impl FactorExecutor {
    /// Pin deployment paths. Python must be absolute and installed; output must
    /// be a canonical runtime-owned directory with mode 0700, separate from all
    /// data namespaces (also checked by the deployment loader). No directories
    /// or processes are created. Outputs never imply successful completion.
    pub fn open(
        python: &Path,
        output: &Path,
        resolver: Arc<EvaluationResolver>,
    ) -> StoreResult<Self> {
        if !python.is_absolute()
            || !python.is_file()
            || !output.is_absolute()
            || std::fs::canonicalize(output).ok().as_deref() != Some(output)
        {
            return Err(StoreError::Invalid("factor worker deployment paths"));
        }
        let metadata = std::fs::symlink_metadata(output)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(StoreError::Invalid("factor output namespace"));
        }
        Ok(Self {
            resolver,
            outputs: LocalArtifacts::open(output)?,
            python: python.to_owned(),
            output: output.to_owned(),
            output_identity: (metadata.dev(), metadata.ino()),
            permits: tokio::sync::Semaphore::new(1),
        })
    }

    pub(super) fn check_output(&self) -> StoreResult<()> {
        let metadata = std::fs::symlink_metadata(&self.output)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o777 != 0o700
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || (metadata.dev(), metadata.ino()) != self.output_identity
        {
            return Err(StoreError::Corrupt("factor output directory changed"));
        }
        Ok(())
    }

    /// Run the fixed numerical subprocess. Internal callers must first authorize
    /// the job and lease and obtain a broker-owned view. Returned output grants
    /// no completion authority; actual files still require sealed verification.
    pub(crate) async fn run(
        &self,
        work: &FactorEvaluationWork,
        view: Option<&Path>,
        timeout: Duration,
    ) -> StoreResult<Option<FactorEvaluationResult>> {
        const MAX_BYTES: usize = 1_048_576;
        self.check_output()?;
        if work.encoded_len() > MAX_BYTES || timeout.is_zero() || timeout > Duration::from_secs(60)
        {
            return Err(StoreError::Invalid("numerical worker budget"));
        }
        tokio::time::timeout(timeout, async {
            let mut command = tokio::process::Command::new(&self.python);
            command
                .args(["-I", "-m", "loop_research.factor_worker"])
                .arg("--view")
                .arg(view.unwrap_or(&self.output))
                .arg("--output")
                .arg(&self.output)
                .env_clear()
                .env("OPENBLAS_NUM_THREADS", "1")
                .env("OMP_NUM_THREADS", "1")
                .current_dir(&self.output)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            if view.is_none() {
                command.arg("--verify-build");
            }
            let mut child = command
                .spawn()
                .map_err(|_| StoreError::Unavailable("factor worker spawn"))?;
            let mut input = child
                .stdin
                .take()
                .ok_or(StoreError::Unavailable("factor worker stdin"))?;
            let output = child
                .stdout
                .take()
                .ok_or(StoreError::Unavailable("factor worker stdout"))?;
            let payload = work.encode_to_vec();
            let write = async {
                input.write_all(&payload).await?;
                input.shutdown().await?;
                drop(input);
                Ok::<_, std::io::Error>(())
            };
            let read = async {
                let mut bytes = Vec::new();
                output
                    .take(MAX_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                Ok::<_, std::io::Error>(bytes)
            };
            let (_, bytes) = tokio::try_join!(write, read)
                .map_err(|_| StoreError::Unavailable("factor worker transport"))?;
            if bytes.len() > MAX_BYTES {
                return Err(StoreError::Invalid("factor worker output bounds"));
            }
            if !child
                .wait()
                .await
                .map_err(|_| StoreError::Unavailable("factor worker exit"))?
                .success()
            {
                return Err(StoreError::Unavailable("factor worker refused execution"));
            }
            self.check_output()?;
            if view.is_none() {
                if !bytes.is_empty() {
                    return Err(StoreError::Corrupt("factor verification output"));
                }
                return Ok(None);
            }
            if bytes.is_empty() {
                return Err(StoreError::Corrupt("factor worker empty output"));
            }
            FactorEvaluationResult::decode(bytes.as_slice())
                .map(Some)
                .map_err(|_| StoreError::Corrupt("factor worker output encoding"))
        })
        .await
        .map_err(|_| StoreError::Unavailable("factor worker deadline"))?
    }
}
