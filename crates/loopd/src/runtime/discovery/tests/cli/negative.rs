use std::path::Path;

use super::*;

#[tokio::test]
async fn unregistered_certificate() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    let mut config: Value = serde_json::from_slice(&std::fs::read(&ops.config).unwrap()).unwrap();
    let certificate = Path::new(config["certificate_file"].as_str().unwrap());
    let directory = certificate.parent().unwrap().to_path_buf();
    // This certificate chains to the trusted CA but has no registered actor.
    // Keeping the original actor metadata cannot manufacture transport identity.
    config["certificate_file"] = serde_json::json!(directory.join("unknown.pem"));
    config["private_key_file"] = serde_json::json!(directory.join("unknown.key"));
    std::fs::write(&ops.config, serde_json::to_vec(&config).unwrap()).unwrap();
    failure(
        ops.cli(&[
            "status",
            "--job",
            submitted["job"]["job_id"].as_str().unwrap(),
        ])
        .await,
        3,
        "authorization",
    );
    assert_eq!(case.calls(), 0);
    ops.stop(false).await;
    case.close().await;
}

#[tokio::test]
async fn timeout_preserves_dispatch() {
    let mut case = Case::open().await;
    let mut ops = case.install();
    case.deploy(&mut ops, false).await;
    let submitted = start(&ops).await;
    case.stall_supplier();
    let job = &submitted["job"];
    let child = ops.spawn_cli(&[
        "--timeout-seconds",
        "3",
        "execute",
        "--job",
        job["job_id"].as_str().unwrap(),
        "--revision",
        job["revision"].as_str().unwrap(),
        "--key",
        "cli.timeout",
    ]);
    called(&case).await;
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    failure(output, 5, "timeout");
    assert_eq!(case.calls(), 1);
    // Execute retains the existing lease/CAS semantics. Retrying the lost
    // command cannot silently refresh its revision or authorize another send.
    failure(
        change(&ops, "execute", job, "cli.timeout").await,
        4,
        "conflict",
    );
    assert_eq!(
        invocation_state(&case, job).await,
        provider::InvocationState::Ambiguous as i32
    );
    let observed = status(&ops, job, 0).await;
    assert_ne!(observed["job"]["status"], "cancelled");
    assert_eq!(case.calls(), 1);
    let events = document(
        ops.cli(&["events", "--job", job["job_id"].as_str().unwrap()])
            .await,
        0,
    );
    assert_eq!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["operation"] == "dispatch")
            .count(),
        1,
    );
    ops.stop(false).await;
    case.close().await;
}
