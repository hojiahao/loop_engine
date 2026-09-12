use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

use prost::Message;
use sha2::{Digest, Sha256};

use crate::runtime::RuntimeDeployment;
use crate::store::AdmissionPolicy;

use super::{NOW, Role, Running, actor};

fn config(running: &Running) -> PathBuf {
    let root = running.fixture.directory.path();
    let actor = actor();
    let job = &running.fixture.job.specification;
    let value = serde_json::json!({
        "schema": "loop.runtime/v1", "bind": "127.0.0.1:8443",
        "server_certificate_file": running.tls.path("server.pem"),
        "server_key_file": running.tls.path("server.key"),
        "client_ca_file": running.tls.path("ca.pem"),
        "identities": [{"actor_id": actor.actor_id.unwrap().value,
            "subject": actor.authenticated_subject, "display_name": actor.display_name,
            "role": "research", "certificate_sha256": [running.tls.client_digest()],
            "not_before_ms": NOW - 1000, "expires_at_ms": NOW + 3_600_000,
            "run_ids": [job.run_id.as_ref().unwrap().value]}],
        "jobs": [{"job_id": job.job_id.as_ref().unwrap().value,
            "specification_sha256": format!("sha256:{:x}", Sha256::digest(job.encode_to_vec()))}],
        "development_store": running.fixture.root, "protected_store": root.join("protected"),
        "view_store": running.views,
        "data": [{"job_id": job.job_id.as_ref().unwrap().value,
            "manifest": running.fixture.context.data, "protected": false}]
    });
    let path = root.join("runtime.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[tokio::test]
async fn startup_loads_actual_tls_and_pinned_namespaces() {
    let running = Running::start(Role::Research).await;
    let deployment = RuntimeDeployment::load(&config(&running)).unwrap();
    assert_eq!(deployment.bind.port(), 8443);
    deployment
        .authority
        .validate_submission(&running.fixture.job.specification)
        .unwrap();
}

#[tokio::test]
async fn startup_denies_public_private_keys() {
    let running = Running::start(Role::Research).await;
    let path = config(&running);
    std::fs::set_permissions(
        running.tls.path("server.key"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(RuntimeDeployment::load(&path).is_err());
}

#[tokio::test]
async fn startup_denies_configuration_symlinks() {
    let running = Running::start(Role::Research).await;
    let path = config(&running);
    let alias = running.fixture.directory.path().join("runtime-link.json");
    symlink(path, &alias).unwrap();
    assert!(RuntimeDeployment::load(&alias).is_err());
}

#[tokio::test]
async fn startup_denies_exposed_protected_sources() {
    let running = Running::start(Role::Research).await;
    let path = config(&running);
    std::fs::set_permissions(
        running.fixture.directory.path().join("protected"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(RuntimeDeployment::load(&path).is_err());
}
