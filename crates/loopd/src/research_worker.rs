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

/// Actual source/environment manifests expected from the installed worker.
/// These are byte identities, not credentials or authority to execute work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResearchBuild {
    /// Canonical `loop.source-files/v1` manifest digest.
    pub source_sha256: [u8; 32],
    /// Canonical `loop.environment-files/v1` manifest digest.
    pub environment_sha256: [u8; 32],
}

/// A trusted server-selected numerical worker, never an LLM-supplied executable.
pub trait PerturbationWorker: Send + Sync {
    /// Declared installed-build contract; concrete workers must verify the
    /// actual running bytes before and after calculation. `None` is unattested
    /// and is refused by the trusted manifest policy, but supports old fixtures.
    fn build_identity(&self) -> Option<ResearchBuild> {
        None
    }

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
    build: Option<ResearchBuild>,
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
            build: None,
        })
    }

    /// Pin actual installed source and numerical dependencies to a frozen
    /// context. Python rehashes them before and after every transition, emitting
    /// no result on drift. This is not kernel/OS attestation or a sandbox.
    /// The unverified constructor remains for explicitly synthetic fixtures.
    pub fn verified(python: &Path, build: ResearchBuild) -> StoreResult<Self> {
        let mut worker = Self::new(python)?;
        worker.build = Some(build);
        Ok(worker)
    }
}

impl PerturbationWorker for PythonPerturber {
    fn build_identity(&self) -> Option<ResearchBuild> {
        self.build
    }

    async fn advance(&self, work: PerturbationWork) -> StoreResult<PerturbationStep> {
        if work.encoded_len() > MAX_BYTES {
            return Err(StoreError::Invalid("perturbation work size"));
        }
        let timeout = Duration::from_secs(if self.build.is_some() { 20 } else { 10 });
        tokio::time::timeout(timeout, async {
            let mut process = tokio::process::Command::new(&self.python);
            process
                .args(["-I", "-m", "loop_research.perturbation"])
                .env_clear();
            if let Some(build) = self.build {
                let hex = |bytes: [u8; 32]| {
                    format!(
                        "sha256:{}",
                        bytes
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    )
                };
                process
                    .env("LOOP_ENGINE_BUILD_SOURCE_SHA256", hex(build.source_sha256))
                    .env(
                        "LOOP_ENGINE_BUILD_ENVIRONMENT_SHA256",
                        hex(build.environment_sha256),
                    );
            }
            let mut child = process
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
