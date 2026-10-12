use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use serde_json::json;

use super::*;

fn config() -> Config {
    serde_json::from_value(document()).unwrap()
}

fn document() -> serde_json::Value {
    json!({"schema":"loop.client/v1","endpoint":"https://localhost:7443",
        "server_name":"localhost","ca_file":"/tmp/ca.pem","certificate_file":"/tmp/client.pem",
        "private_key_file":"/tmp/client.key",
        "actor":{"actor_id":"agent.discovery","subject":"spiffe://loop/discovery","display_name":"Discovery"}})
}

fn file(root: &Path, name: &str, mode: u32) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, b"private material").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn accepts_https() {
    assert!(config().validate(Profile::Discovery).is_ok());
}

#[test]
fn rejects_endpoints() {
    for endpoint in [
        "http://localhost",
        "https://user:secret@localhost",
        "https://localhost?secret=x",
        "https://localhost#secret",
        "https://localhost/api",
        " https://localhost",
        "https://@localhost",
    ] {
        let mut value = config();
        value.endpoint = endpoint.into();
        assert_eq!(
            value.validate(Profile::Discovery),
            Err(Failure::Configuration)
        );
    }
}

#[test]
fn rejects_server_names() {
    for name in [
        "",
        "https://localhost",
        "local host",
        "*.localhost",
        "localhost/",
        "a..b",
    ] {
        let mut value = config();
        value.server_name = name.into();
        assert_eq!(
            value.validate(Profile::Discovery),
            Err(Failure::Configuration)
        );
    }
}

#[test]
fn rejects_unknown_config() {
    let mut value = document();
    value["tls_insecure"] = json!(true);
    assert!(serde_json::from_value::<Config>(value).is_err());
}

#[test]
fn rejects_actor_kind() {
    let mut value = document();
    value["actor"]["kind"] = json!("human");
    assert!(serde_json::from_value::<Config>(value).is_err());
}

#[test]
fn rejects_duplicate_config() {
    let mut value = serde_json::to_string(&document()).unwrap();
    value.insert_str(1, "\"schema\":\"loop.client/v1\",");
    assert!(serde_json::from_str::<Config>(&value).is_err());
}

#[test]
fn rejects_actor_controls() {
    let mut value = config();
    value.actor.subject = "spoofed\nsubject".into();
    assert_eq!(
        value.validate(Profile::Discovery),
        Err(Failure::Configuration)
    );
}

#[test]
fn isolates_operator_profile() {
    let mut value = config();
    assert_eq!(
        value.validate(Profile::Operator),
        Err(Failure::Configuration)
    );
    value.schema = "loop.operator/v1".into();
    assert!(value.validate(Profile::Operator).is_ok());
    assert_eq!(
        value.validate(Profile::Discovery),
        Err(Failure::Configuration)
    );
    assert_eq!(Profile::Operator.actor_kind(), v1::ActorKind::Human);
    assert_eq!(Profile::Discovery.actor_kind(), v1::ActorKind::Agent);
}

#[test]
fn reads_private_file() {
    let root = tempfile::tempdir().unwrap();
    let path = file(root.path(), "client.json", 0o600);
    assert_eq!(read_file(&path, true, 128).unwrap(), b"private material");
}

#[test]
fn rejects_public_secret() {
    let root = tempfile::tempdir().unwrap();
    let path = file(root.path(), "client.key", 0o640);
    assert_eq!(read_file(&path, true, 128), Err(Failure::Configuration));
}

#[test]
fn rejects_writable_certificate() {
    let root = tempfile::tempdir().unwrap();
    let path = file(root.path(), "client.pem", 0o664);
    assert_eq!(read_file(&path, false, 128), Err(Failure::Configuration));
}

#[test]
fn rejects_symlink_file() {
    let root = tempfile::tempdir().unwrap();
    let path = file(root.path(), "client.key", 0o600);
    let link = root.path().join("link");
    symlink(path, &link).unwrap();
    assert_eq!(read_file(&link, true, 128), Err(Failure::Configuration));
}

#[test]
fn rejects_symlink_parent() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("real");
    fs::create_dir(&directory).unwrap();
    file(&directory, "client.key", 0o600);
    let link = root.path().join("link");
    symlink(directory, &link).unwrap();
    assert_eq!(
        read_file(&link.join("client.key"), true, 128),
        Err(Failure::Configuration)
    );
}

#[test]
fn rejects_large_file() {
    let root = tempfile::tempdir().unwrap();
    let path = file(root.path(), "input.pb", 0o600);
    assert_eq!(read_file(&path, false, 2), Err(Failure::Configuration));
}

#[test]
fn rejects_empty_file() {
    let root = tempfile::tempdir().unwrap();
    let path = file(root.path(), "input.pb", 0o600);
    fs::write(&path, []).unwrap();
    assert_eq!(read_file(&path, false, 128), Err(Failure::Configuration));
}

#[test]
fn rejects_directory() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        read_file(root.path(), true, 128),
        Err(Failure::Configuration)
    );
}

#[test]
fn rejects_fifo() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fifo");
    rustix::fs::mkfifoat(rustix::fs::CWD, &path, Mode::RUSR | Mode::WUSR).unwrap();
    assert_eq!(read_file(&path, true, 128), Err(Failure::Configuration));
}

#[test]
fn rejects_relative_path() {
    assert_eq!(
        read_file(Path::new("client.json"), true, 128),
        Err(Failure::Configuration)
    );
}

#[test]
fn rejects_dot_path() {
    let root = tempfile::tempdir().unwrap();
    file(root.path(), "client.key", 0o600);
    let path = root.path().join(".").join("client.key");
    assert_eq!(read_file(&path, true, 128), Err(Failure::Configuration));
}
