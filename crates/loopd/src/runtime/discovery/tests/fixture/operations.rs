use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Output, Stdio};
use std::time::Duration;

use prost::Message;
use rustix::process::{Pid, Signal, kill_process};
use serde_json::json;
use tempfile::TempDir;
use tokio::process::{Child, Command};

use super::{Case, private_dir, private_file};
use crate::test_support;

pub(in crate::runtime::discovery::tests) struct Operations {
    _directory: TempDir,
    pub(in crate::runtime::discovery::tests) config: PathBuf,
    pub(in crate::runtime::discovery::tests) input: PathBuf,
    loopctl: PathBuf,
    loopd: PathBuf,
    runtime: PathBuf,
    database: PathBuf,
    server: Option<Child>,
    run_deployment: Option<serde_json::Value>,
    run_config: PathBuf,
    run_reference: PathBuf,
}

impl Case {
    pub(in crate::runtime::discovery::tests) fn stall_supplier(&self) {
        private_file(&self.directory.path().join("supplier-delay"), b"30000");
    }

    pub(in crate::runtime::discovery::tests) fn install(&self) -> Operations {
        let directory = tempfile::Builder::new()
            .prefix("loop-discovery-install-")
            .tempdir()
            .unwrap();
        let bin = directory.path().join("bin");
        private_dir(&bin);
        let executable = std::env::current_exe().unwrap();
        let profile = executable
            .parent()
            .and_then(|path| path.parent())
            .expect("test executable must be inside the Cargo profile deps directory");
        for name in ["loopctl", "loopd"] {
            let source = profile.join(name);
            assert!(
                source.is_file(),
                "build loopctl and loopd binaries before running operational acceptance"
            );
            let target = bin.join(name);
            fs::copy(source, &target).unwrap();
            fs::set_permissions(target, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = directory.path().join("client.json");
        let input = directory.path().join("input.binpb");
        private_file(&input, &self.input.encode_to_vec());
        private_file(
            &config,
            &serde_json::to_vec(&json!({
                "schema": "loop.client/v1",
                "endpoint": format!("https://localhost:{}", self.address.port()),
                "server_name": "localhost",
                "ca_file": self.tls.path("ca.pem"),
                "certificate_file": self.tls.path("client.pem"),
                "private_key_file": self.tls.path("client.key"),
                "actor": {
                    "actor_id": "agent.discovery",
                    "subject": "agent:discovery",
                    "display_name": "Discovery fixture"
                }
            }))
            .unwrap(),
        );
        Operations {
            config,
            input,
            loopctl: bin.join("loopctl"),
            loopd: bin.join("loopd"),
            runtime: directory.path().join("runtime.json"),
            database: directory.path().join("database-url"),
            server: None,
            run_deployment: None,
            run_config: directory.path().join("operator.json"),
            run_reference: directory.path().join("run-reference.binpb"),
            _directory: directory,
        }
    }

    pub(in crate::runtime::discovery::tests) async fn deploy(
        &mut self,
        operations: &mut Operations,
        stop_only: bool,
    ) {
        if !self.task.is_finished() {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
        operations.stop(false).await;
        let mut deployment = self.deployment();
        deployment["bind"] = self.address.to_string().into();
        if let Some(run) = &operations.run_deployment {
            deployment["identities"]
                .as_array_mut()
                .unwrap()
                .push(run["identity"].clone());
            if !stop_only {
                deployment["runs"] = run["runs"].clone();
            }
        }
        if stop_only {
            deployment.as_object_mut().unwrap().remove("discovery");
        }
        private_file(
            &operations.runtime,
            &serde_json::to_vec(&deployment).unwrap(),
        );
        private_file(&operations.database, test_support::test_url().as_bytes());
        let schema = test_support::schema(&self.directory.path().join("state"));
        let server = Command::new(&operations.loopd)
            .current_dir(operations._directory.path())
            .arg("--database-url-file")
            .arg(&operations.database)
            .arg("--database-schema")
            .arg(schema)
            .arg("--bind")
            .arg("127.0.0.1:0")
            .arg("--runtime-config")
            .arg(&operations.runtime)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("installed loopd must start");
        operations.server = Some(server);
        let ready = async {
            loop {
                let server = operations.server.as_mut().unwrap();
                assert!(
                    server.try_wait().unwrap().is_none(),
                    "installed loopd exited before opening its authenticated endpoint"
                );
                if tokio::net::TcpStream::connect(self.address).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(30), ready)
            .await
            .expect("installed loopd did not become ready within 30 seconds");
    }
}

impl Operations {
    pub(super) fn directory(&self) -> &std::path::Path {
        self._directory.path()
    }

    pub(super) fn configure_run(
        &mut self,
        deployment: serde_json::Value,
        config: &serde_json::Value,
        policy: &loop_protocol::wire::v1::PolicyReference,
    ) {
        private_file(&self.run_config, &serde_json::to_vec(config).unwrap());
        private_file(&self.run_reference, &policy.encode_to_vec());
        self.run_deployment = Some(deployment);
    }

    pub(in crate::runtime::discovery::tests) async fn run_cli(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.loopctl);
        command
            .current_dir(self._directory.path())
            .arg("run")
            .arg("--config")
            .arg(&self.run_config)
            .arg("--timeout-seconds")
            .arg("30");
        for argument in args {
            if *argument == "PLAN" {
                command.arg(&self.run_reference);
            } else {
                command.arg(argument);
            }
        }
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(35), child.wait_with_output())
            .await
            .unwrap()
            .unwrap()
    }

    pub(in crate::runtime::discovery::tests) async fn cli(&self, args: &[&str]) -> Output {
        let child = self.spawn_cli(args);
        tokio::time::timeout(Duration::from_secs(35), child.wait_with_output())
            .await
            .expect("installed loopctl exceeded its test deadline")
            .expect("installed loopctl output must be readable")
    }

    pub(in crate::runtime::discovery::tests) fn spawn_cli(&self, args: &[&str]) -> Child {
        let mut command = Command::new(&self.loopctl);
        command.current_dir(self._directory.path());
        command.arg("discovery").arg("--config").arg(&self.config);
        if !args.iter().any(|argument| {
            *argument == "--timeout-seconds" || argument.starts_with("--timeout-seconds=")
        }) {
            command.arg("--timeout-seconds").arg("30");
        }
        for argument in args {
            if *argument == "INPUT" {
                command.arg(&self.input);
            } else {
                command.arg(argument);
            }
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("installed loopctl must start")
    }

    pub(in crate::runtime::discovery::tests) async fn stop(
        &mut self,
        kill: bool,
    ) -> Option<ExitStatus> {
        let mut server = self.server.take()?;
        if server.try_wait().unwrap().is_none() {
            if kill {
                server.start_kill().unwrap();
            } else {
                let pid = server
                    .id()
                    .and_then(|id| i32::try_from(id).ok())
                    .and_then(Pid::from_raw)
                    .unwrap();
                kill_process(pid, Signal::TERM).unwrap();
            }
        }
        let status = tokio::time::timeout(Duration::from_secs(10), server.wait())
            .await
            .expect("installed loopd did not stop within 10 seconds")
            .expect("installed loopd must be reaped");
        assert!(
            kill || status.success(),
            "loopd must handle SIGTERM cleanly"
        );
        Some(status)
    }
}

impl Drop for Operations {
    fn drop(&mut self) {
        if let Some(server) = &mut self.server {
            let _ = server.start_kill();
        }
    }
}
