//! Bounded transport to the installed numerical worker; no numerical logic.
#![deny(missing_docs)]

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use loop_protocol::wire::v1::{PerturbationStep, PerturbationWork};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::store::{StoreError, StoreResult};

const MAX_BYTES: usize = 1_048_576;

/// A trusted server-selected numerical worker, never an LLM-supplied executable.
pub trait PerturbationWorker: Send + Sync {
    /// Calculate one bounded transition without persistent or external effects.
    /// Errors, cancellation and timeouts must not publish partial state.
    fn advance(
        &self,
        work: PerturbationWork,
    ) -> impl Future<Output = StoreResult<PerturbationStep>> + Send;
}

/// Invoke the fixed installed Python module with Protobuf stdin/stdout.
pub struct PythonPerturber {
    python: PathBuf,
}

impl PythonPerturber {
    /// Select an absolute deployment-owned Python executable, not a request path.
    /// The deployment must pin its environment/source in the research context.
    /// Returns an error for relative paths or missing/non-file executables.
    pub fn new(python: &Path) -> StoreResult<Self> {
        if !python.is_absolute() || !python.is_file() {
            return Err(StoreError::Invalid("research Python executable"));
        }
        Ok(Self {
            python: python.to_owned(),
        })
    }
}

impl PerturbationWorker for PythonPerturber {
    async fn advance(&self, work: PerturbationWork) -> StoreResult<PerturbationStep> {
        if work.encoded_len() > MAX_BYTES {
            return Err(StoreError::Invalid("perturbation work size"));
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut child = tokio::process::Command::new(&self.python)
                .args(["-I", "-m", "loop_research.perturbation"])
                .env_clear()
                .env("OPENBLAS_NUM_THREADS", "1")
                .env("OMP_NUM_THREADS", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| StoreError::Unavailable("research worker spawn"))?;
            let mut input = child
                .stdin
                .take()
                .ok_or(StoreError::Unavailable("worker stdin"))?;
            let output = child
                .stdout
                .take()
                .ok_or(StoreError::Unavailable("worker stdout"))?;
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
                .map_err(|_| StoreError::Unavailable("research worker transport"))?;
            if bytes.len() > MAX_BYTES {
                return Err(StoreError::Invalid("research worker output size"));
            }
            let status = child
                .wait()
                .await
                .map_err(|_| StoreError::Unavailable("worker exit"))?;
            if status.code() == Some(2) {
                return Err(StoreError::Invalid("research worker rejected input"));
            }
            if !status.success() {
                return Err(StoreError::Unavailable(
                    "research worker exited unsuccessfully",
                ));
            }
            if bytes.is_empty() {
                return Err(StoreError::Invalid("research worker output size"));
            }
            PerturbationStep::decode(bytes.as_slice())
                .map_err(|_| StoreError::Invalid("research worker output"))
        })
        .await
        .map_err(|_| StoreError::Unavailable("research worker timeout"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_must_be_absolute() {
        assert!(matches!(
            PythonPerturber::new(Path::new("python")),
            Err(StoreError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn process_failure_is_infrastructure_error() {
        let worker = PythonPerturber::new(Path::new("/bin/false")).unwrap();
        assert!(matches!(
            worker.advance(PerturbationWork::default()).await,
            Err(StoreError::Unavailable(_))
        ));
    }
}
