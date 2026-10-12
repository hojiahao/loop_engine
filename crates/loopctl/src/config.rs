use std::fs::{File, Metadata};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use loop_protocol::wire::v1;
use rustix::fs::{Mode, OFlags, open, openat};
use serde::Deserialize;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

use crate::output::Failure;

const FILE_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK)
    .union(OFlags::CLOEXEC);

// These types deliberately have no Debug implementation: paths and principal
// metadata must never become diagnostics alongside TLS material.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: String,
    endpoint: String,
    server_name: String,
    ca_file: PathBuf,
    certificate_file: PathBuf,
    private_key_file: PathBuf,
    actor: Actor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Actor {
    actor_id: String,
    subject: String,
    display_name: String,
}

pub(crate) struct Connection {
    pub(crate) endpoint: Endpoint,
    pub(crate) actor: v1::Actor,
}

#[derive(Clone, Copy)]
pub(crate) enum Profile {
    Discovery,
    Operator,
}

impl Profile {
    fn schema(self) -> &'static str {
        match self {
            Self::Discovery => "loop.client/v1",
            Self::Operator => "loop.operator/v1",
        }
    }

    fn actor_kind(self) -> v1::ActorKind {
        match self {
            Self::Discovery => v1::ActorKind::Agent,
            Self::Operator => v1::ActorKind::Human,
        }
    }
}

impl Connection {
    pub(crate) fn load(path: &Path, timeout: Duration, profile: Profile) -> Result<Self, Failure> {
        let bytes = read_file(path, true, 65_536)?;
        let config: Config = serde_json::from_slice(&bytes).map_err(|_| Failure::Configuration)?;
        config.validate(profile)?;
        let ca = read_file(&config.ca_file, false, 131_072)?;
        let certificate = read_file(&config.certificate_file, false, 131_072)?;
        let key = read_file(&config.private_key_file, true, 131_072)?;
        let endpoint =
            Endpoint::from_shared(config.endpoint).map_err(|_| Failure::Configuration)?;
        if endpoint.uri().path() != "/" {
            return Err(Failure::Configuration);
        }
        let endpoint = endpoint
            .connect_timeout(timeout.min(Duration::from_secs(5)))
            .timeout(timeout)
            .tls_config(
                ClientTlsConfig::new()
                    .domain_name(config.server_name)
                    .ca_certificate(Certificate::from_pem(ca))
                    .identity(Identity::from_pem(certificate, key)),
            )
            .map_err(|_| Failure::Configuration)?;
        Ok(Self {
            endpoint,
            actor: v1::Actor {
                actor_id: Some(v1::ActorId {
                    value: config.actor.actor_id,
                }),
                kind: profile.actor_kind() as i32,
                display_name: config.actor.display_name,
                authenticated_subject: config.actor.subject,
            },
        })
    }
}

impl Config {
    fn validate(&self, profile: Profile) -> Result<(), Failure> {
        let endpoint = url::Url::parse(&self.endpoint).map_err(|_| Failure::Configuration)?;
        if self.schema != profile.schema()
            || !self.endpoint.starts_with("https://")
            || self.endpoint.chars().any(char::is_whitespace)
            || endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || self.endpoint.contains('@')
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
            || !server_name(&self.server_name)
            || !valid_id(&self.actor.actor_id)
            || !bounded_text(&self.actor.subject, 512)
            || (!self.actor.display_name.is_empty() && !bounded_text(&self.actor.display_name, 256))
        {
            return Err(Failure::Configuration);
        }
        Ok(())
    }
}

pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn bounded_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn server_name(value: &str) -> bool {
    if value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

pub(crate) fn read_file(path: &Path, private: bool, maximum: u64) -> Result<Vec<u8>, Failure> {
    if !path.is_absolute()
        || path.as_os_str().len() > 4_096
        || std::fs::canonicalize(path)
            .ok()
            .is_none_or(|canonical| canonical.as_os_str() != path.as_os_str())
    {
        return Err(Failure::Configuration);
    }
    // Open every directory without following links. A canonicalize/open race
    // cannot replace an intermediate directory with a symlink to another tree.
    let mut descriptor = open("/", FILE_FLAGS | OFlags::DIRECTORY, Mode::empty())
        .map_err(|_| Failure::Configuration)?;
    let mut components = path.components().skip(1).peekable();
    if components.peek().is_none() {
        return Err(Failure::Configuration);
    }
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(Failure::Configuration);
        };
        let flags = if components.peek().is_some() {
            FILE_FLAGS | OFlags::DIRECTORY
        } else {
            FILE_FLAGS
        };
        descriptor =
            openat(&descriptor, name, flags, Mode::empty()).map_err(|_| Failure::Configuration)?;
    }
    let mut file = File::from(descriptor);
    let before = file.metadata().map_err(|_| Failure::Configuration)?;
    let uid = rustix::process::geteuid().as_raw();
    if !before.is_file()
        || before.size() == 0
        || before.size() > maximum
        || before.mode() & (if private { 0o077 } else { 0o022 }) != 0
        || (before.uid() != 0 && before.uid() != uid)
    {
        return Err(Failure::Configuration);
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Configuration)?;
    for after in [file.metadata(), std::fs::symlink_metadata(path)] {
        let after = after.map_err(|_| Failure::Configuration)?;
        if !after.is_file()
            || bytes.len() as u64 != before.size()
            || fingerprint(&before) != fingerprint(&after)
        {
            return Err(Failure::Configuration);
        }
    }
    Ok(bytes)
}

fn fingerprint(metadata: &Metadata) -> (u64, u64, u64, u32, u32, i64, i64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.size(),
        metadata.uid(),
        metadata.mode(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

#[cfg(test)]
mod tests;
