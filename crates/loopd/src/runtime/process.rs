//! Fixed-worker supervision; cancellation kills the uv process and its worker.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::store::{StoreError, StoreResult};

struct ProcessGroup(Option<Pid>);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            let _ = kill_process_group(group, Signal::KILL);
        }
    }
}

pub(super) fn command(executable: &Path, directory: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("OPENBLAS_NUM_THREADS", "1")
        .env("OMP_NUM_THREADS", "1")
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .process_group(0);
    command
}

pub(super) async fn run(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
    codes: &[i32],
) -> StoreResult<(i32, Vec<u8>)> {
    if input.len() > 1_048_576 || timeout.is_zero() || timeout > Duration::from_secs(180) {
        return Err(StoreError::Invalid("validation subprocess budget"));
    }
    tokio::time::timeout(timeout, async {
        let mut child = command
            .spawn()
            .map_err(|_| StoreError::Unavailable("validation spawn"))?;
        let mut group = ProcessGroup(
            child
                .id()
                .and_then(|id| i32::try_from(id).ok())
                .and_then(Pid::from_raw),
        );
        if group.0.is_none() {
            return Err(StoreError::Unavailable("validation process group"));
        }
        let mut stdin = child
            .stdin
            .take()
            .ok_or(StoreError::Unavailable("validation stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(StoreError::Unavailable("validation stdout"))?;
        let write = async {
            stdin.write_all(input).await?;
            stdin.shutdown().await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let read = async {
            let mut bytes = Vec::new();
            stdout.take(1_048_577).read_to_end(&mut bytes).await?;
            Ok::<_, std::io::Error>(bytes)
        };
        let (_, output) = tokio::try_join!(write, read)
            .map_err(|_| StoreError::Unavailable("validation transport"))?;
        if output.len() > 1_048_576 {
            return Err(StoreError::Corrupt("validation stdout bound"));
        }
        let status = child
            .wait()
            .await
            .map_err(|_| StoreError::Unavailable("validation wait"))?;
        // These deployment-pinned workers wait for their children and never
        // daemonize. Do not signal a reaped process ID on ordinary completion.
        group.0 = None;
        let code =
            status
                .code()
                .filter(|code| codes.contains(code))
                .ok_or(StoreError::Unavailable(
                    "validation worker refused execution",
                ))?;
        if output.is_empty() {
            return Err(StoreError::Corrupt("validation stdout bound"));
        }
        Ok((code, output))
    })
    .await
    .map_err(|_| StoreError::Unavailable("validation worker deadline"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.venv/bin/python")
    }

    fn process_state(stat: &str) -> Option<&str> {
        // Linux comm is parenthesized and may contain whitespace or ')'. The
        // state field follows the final closing parenthesis, not the third word.
        stat.rsplit_once(") ")?.1.split_whitespace().next()
    }

    #[test]
    fn zombie_state_parses() {
        assert_eq!(process_state("42 (python worker) Z 1 42"), Some("Z"));
        assert_eq!(process_state("42 (python ) worker) Z 1 42"), Some("Z"));
        assert_eq!(process_state("42 (python Z worker) S 1 42"), Some("S"));
        assert_eq!(process_state("cpu 1 2 3"), None);
    }

    #[tokio::test]
    async fn isolated_environment() {
        let directory = tempfile::Builder::new()
            .prefix("loop-validation-env-")
            .tempdir()
            .unwrap();
        let mut worker = command(&python(), directory.path());
        worker.args(["-I", "-c", "import json,os,sys; print(json.dumps({'environment':dict(os.environ),'input':sys.stdin.read()}))"]);
        let (_, bytes) = run(&mut worker, b"request", Duration::from_secs(5), &[0])
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["input"], "request");
        for name in value["environment"].as_object().unwrap().keys() {
            assert!(
                [
                    "PATH",
                    "LANG",
                    "LC_CTYPE",
                    "OPENBLAS_NUM_THREADS",
                    "OMP_NUM_THREADS"
                ]
                .contains(&name.as_str()),
                "unexpected inherited environment: {name}"
            );
        }
    }

    #[tokio::test]
    async fn cancelled_descendant_dies() {
        let directory = tempfile::Builder::new()
            .prefix("loop-validation-cancel-")
            .tempdir()
            .unwrap();
        let path = directory.path().join("child.pid");
        // Reproduce the publication gap in Python's write_text: file creation
        // does not mean the child PID has been written. An empty PID would turn
        // /proc/{pid}/stat into /proc//stat, which is the host's CPU statistics.
        std::fs::write(&path, "").unwrap();
        let mut worker = command(&python(), directory.path());
        worker
            .args([
                "-I",
                "-c",
                concat!(
                    "import pathlib,subprocess,sys,time; ",
                    "p=subprocess.Popen([sys.executable,'-I','-c','import time;time.sleep(30)']); ",
                    "path=pathlib.Path(sys.argv[1]); ready=path.with_suffix('.ready'); ",
                    "ready.write_text(str(p.pid)); ready.replace(path); ",
                    "print('started',flush=True); time.sleep(30)",
                ),
            ])
            .arg(&path);
        let task =
            tokio::spawn(async move { run(&mut worker, b"", Duration::from_secs(10), &[0]).await });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let pid = loop {
            let contents = std::fs::read_to_string(&path).unwrap();
            if !contents.is_empty() {
                let pid = contents.parse::<u32>().expect("invalid child PID");
                assert!(pid > 1, "invalid child PID");
                break pid;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "worker did not start"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
            if stat
                .as_ref()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                || stat
                    .as_ref()
                    .is_ok_and(|stat| process_state(stat) == Some("Z"))
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "cancelled descendant remains alive"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn worker_failure_classified() {
        let directory = tempfile::Builder::new()
            .prefix("loop-validation-failure-")
            .tempdir()
            .unwrap();
        let mut worker = command(&python(), directory.path());
        worker.args(["-I", "-c", "raise SystemExit(1)"]);
        assert!(matches!(
            run(&mut worker, b"", Duration::from_secs(5), &[0]).await,
            Err(StoreError::Unavailable(
                "validation worker refused execution"
            ))
        ));
    }

    #[tokio::test]
    async fn empty_output_denied() {
        let directory = tempfile::Builder::new()
            .prefix("loop-validation-empty-")
            .tempdir()
            .unwrap();
        let mut worker = command(&python(), directory.path());
        worker.args(["-I", "-c", "pass"]);
        assert!(matches!(
            run(&mut worker, b"", Duration::from_secs(5), &[0]).await,
            Err(StoreError::Corrupt("validation stdout bound"))
        ));
    }

    #[tokio::test]
    async fn output_budget_enforced() {
        let directory = tempfile::Builder::new()
            .prefix("loop-validation-limit-")
            .tempdir()
            .unwrap();
        let mut worker = command(&python(), directory.path());
        worker.args(["-I", "-c", "print('x'*1048577)"]);
        assert!(matches!(
            run(&mut worker, b"", Duration::from_secs(5), &[0]).await,
            Err(StoreError::Corrupt("validation stdout bound"))
        ));
    }
}
