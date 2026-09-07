//! Fail-closed validation for immutable artifact references.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::wire::v1::ArtifactRef;

pub const MAX_ARTIFACT_URI_BYTES: usize = 2_048;
const CONTENT_ADDRESS_PREFIX: &str = "artifact://sha256/";
const MIN_TIMESTAMP_SECONDS: i64 = -62_135_596_800;
const MAX_TIMESTAMP_SECONDS: i64 = 253_402_300_799;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactValidationCode {
    MissingField,
    InvalidDigest,
    InvalidLocator,
    UriTooLong,
    IdentityMismatch,
    InvalidSchema,
    InvalidMediaType,
    InvalidTimestamp,
}

impl ArtifactValidationCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingField => "missing_field",
            Self::InvalidDigest => "invalid_digest",
            Self::InvalidLocator => "invalid_locator",
            Self::UriTooLong => "uri_too_long",
            Self::IdentityMismatch => "identity_mismatch",
            Self::InvalidSchema => "invalid_schema",
            Self::InvalidMediaType => "invalid_media_type",
            Self::InvalidTimestamp => "invalid_timestamp",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactValidationError {
    pub code: ArtifactValidationCode,
    pub field: &'static str,
}

impl ArtifactValidationError {
    const fn new(code: ArtifactValidationCode, field: &'static str) -> Self {
        Self { code, field }
    }
}

impl Display for ArtifactValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} failed artifact validation ({})",
            self.field,
            self.code.as_str()
        )
    }
}

impl Error for ArtifactValidationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedArtifactRef {
    pub artifact_id: String,
    pub uri: String,
    pub sha256: [u8; 32],
    pub schema_name: String,
    pub schema_version: u32,
    pub schema_sha256: [u8; 32],
    pub media_type: String,
    pub byte_size: u64,
    pub row_count: Option<u64>,
    pub created_at_seconds: i64,
    pub created_at_nanos: i32,
    pub manifest_sha256: Option<[u8; 32]>,
}

/// Validate and detach an `ArtifactRef` before it enters a domain or storage
/// layer. Only the internal, credential-free content-address scheme is valid;
/// storage backends resolve it under their own service identity.
pub fn validate_artifact_ref(
    reference: &ArtifactRef,
) -> Result<ValidatedArtifactRef, ArtifactValidationError> {
    let digest = require_digest(reference.sha256.as_ref(), "sha256")?;
    let digest_hex = lower_hex(&digest);

    if reference.uri.len() > MAX_ARTIFACT_URI_BYTES {
        return Err(ArtifactValidationError::new(
            ArtifactValidationCode::UriTooLong,
            "uri",
        ));
    }
    let expected_uri = format!("{CONTENT_ADDRESS_PREFIX}{digest_hex}");
    if !is_strict_content_address(&reference.uri) {
        return Err(ArtifactValidationError::new(
            ArtifactValidationCode::InvalidLocator,
            "uri",
        ));
    }
    if reference.uri != expected_uri {
        return Err(ArtifactValidationError::new(
            ArtifactValidationCode::IdentityMismatch,
            "uri",
        ));
    }

    let artifact_id = reference
        .artifact_id
        .as_ref()
        .ok_or_else(|| {
            ArtifactValidationError::new(ArtifactValidationCode::MissingField, "artifact_id")
        })?
        .value
        .as_str();
    let expected_artifact_id = format!("sha256:{digest_hex}");
    if artifact_id != expected_artifact_id {
        return Err(ArtifactValidationError::new(
            ArtifactValidationCode::IdentityMismatch,
            "artifact_id",
        ));
    }

    let schema = reference.schema.as_ref().ok_or_else(|| {
        ArtifactValidationError::new(ArtifactValidationCode::MissingField, "schema")
    })?;
    if schema.version == 0 || !is_identifier(&schema.name) {
        return Err(ArtifactValidationError::new(
            ArtifactValidationCode::InvalidSchema,
            "schema",
        ));
    }
    let schema_digest = require_digest(schema.schema_sha256.as_ref(), "schema.schema_sha256")?;
    if !is_media_type(&reference.media_type) {
        return Err(ArtifactValidationError::new(
            ArtifactValidationCode::InvalidMediaType,
            "media_type",
        ));
    }
    let (created_at_seconds, created_at_nanos) =
        require_timestamp(reference.created_at.as_ref(), "created_at")?;
    let manifest_sha256 = reference
        .manifest_sha256
        .as_ref()
        .map(|value| require_digest(Some(value), "manifest_sha256"))
        .transpose()?;

    Ok(ValidatedArtifactRef {
        artifact_id: artifact_id.to_owned(),
        uri: reference.uri.clone(),
        sha256: digest,
        schema_name: schema.name.clone(),
        schema_version: schema.version,
        schema_sha256: schema_digest,
        media_type: reference.media_type.clone(),
        byte_size: reference.byte_size,
        row_count: reference.row_count,
        created_at_seconds,
        created_at_nanos,
        manifest_sha256,
    })
}

fn require_timestamp(
    timestamp: Option<&prost_types::Timestamp>,
    field: &'static str,
) -> Result<(i64, i32), ArtifactValidationError> {
    let timestamp = timestamp
        .ok_or_else(|| ArtifactValidationError::new(ArtifactValidationCode::MissingField, field))?;
    if timestamp.seconds < MIN_TIMESTAMP_SECONDS
        || timestamp.seconds > MAX_TIMESTAMP_SECONDS
        || timestamp.nanos < 0
        || timestamp.nanos >= 1_000_000_000
    {
        return Err(ArtifactValidationError::new(
            ArtifactValidationCode::InvalidTimestamp,
            field,
        ));
    }
    Ok((timestamp.seconds, timestamp.nanos))
}

fn require_digest(
    digest: Option<&crate::wire::v1::Sha256Digest>,
    field: &'static str,
) -> Result<[u8; 32], ArtifactValidationError> {
    let bytes = digest
        .ok_or_else(|| ArtifactValidationError::new(ArtifactValidationCode::MissingField, field))?
        .value
        .as_slice();
    bytes
        .try_into()
        .map_err(|_| ArtifactValidationError::new(ArtifactValidationCode::InvalidDigest, field))
}

fn is_strict_content_address(uri: &str) -> bool {
    let Some(hex) = uri.strip_prefix(CONTENT_ADDRESS_PREFIX) else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn lower_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.split('.').all(|segment| {
            let mut bytes = segment.bytes();
            bytes.next().is_some_and(|first| first.is_ascii_lowercase())
                && bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

fn is_media_type(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.is_ascii()
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
        && value.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty() && !subtype.is_empty() && !subtype.contains('/')
        })
}

#[cfg(test)]
mod tests {
    use prost::Message;
    use prost_types::{FileDescriptorSet, Timestamp};

    use super::*;
    use crate::wire::v1::{ArtifactId, ArtifactSchemaReference, Sha256Digest};

    const VECTORS: &str = include_str!("../../../tests/contracts/artifact_ref_vectors.tsv");

    struct Vector<'a> {
        name: &'a str,
        expected: &'a str,
        uri: String,
        digest: Vec<u8>,
        artifact_id: String,
        created_at: &'a str,
    }

    #[test]
    fn shared_artifact_vectors_fail_closed() {
        for vector in vectors() {
            let reference = reference(&vector);
            match validate_artifact_ref(&reference) {
                Ok(validated) => {
                    assert_eq!(vector.expected, "accept", "{}", vector.name);
                    assert_eq!(validated.uri, vector.uri);
                    let created_at = reference
                        .created_at
                        .as_ref()
                        .expect("accepted vector must contain created_at");
                    assert_eq!(
                        (validated.created_at_seconds, validated.created_at_nanos),
                        (created_at.seconds, created_at.nanos),
                        "{}",
                        vector.name
                    );
                }
                Err(error) => assert_eq!(error.code.as_str(), vector.expected, "{}", vector.name),
            }
        }
    }

    #[test]
    fn artifact_wire_contract_has_no_inline_payload_field() {
        let descriptor = FileDescriptorSet::decode(crate::FILE_DESCRIPTOR_SET)
            .expect("committed descriptor must decode");
        let file = descriptor
            .file
            .iter()
            .find(|file| file.name.as_deref() == Some("loop/v1/artifact.proto"))
            .expect("artifact descriptor must exist");
        let message = file
            .message_type
            .iter()
            .find(|message| message.name.as_deref() == Some("ArtifactRef"))
            .expect("ArtifactRef descriptor must exist");

        assert!(message.field.iter().all(|field| {
            field.r#type != Some(prost_types::field_descriptor_proto::Type::Bytes as i32)
        }));
        assert!(message.field.iter().all(|field| {
            !matches!(
                field.name.as_deref(),
                Some("bytes" | "data" | "payload" | "content" | "inline_bytes")
            )
        }));
    }

    fn reference(vector: &Vector<'_>) -> ArtifactRef {
        ArtifactRef {
            artifact_id: Some(ArtifactId {
                value: vector.artifact_id.clone(),
            }),
            uri: vector.uri.clone(),
            sha256: Some(Sha256Digest {
                value: vector.digest.clone(),
            }),
            schema: Some(ArtifactSchemaReference {
                name: "table.factor_values".to_owned(),
                version: 1,
                schema_sha256: Some(Sha256Digest { value: vec![2; 32] }),
            }),
            media_type: "application/vnd.apache.parquet".to_owned(),
            byte_size: 42,
            row_count: Some(1),
            created_at: match vector.created_at {
                "valid" => Some(Timestamp {
                    seconds: 1,
                    nanos: 0,
                }),
                "missing" => None,
                "zero" => Some(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
                "before_minimum" => Some(Timestamp {
                    seconds: MIN_TIMESTAMP_SECONDS - 1,
                    nanos: 0,
                }),
                "after_maximum" => Some(Timestamp {
                    seconds: MAX_TIMESTAMP_SECONDS + 1,
                    nanos: 0,
                }),
                "negative_nanos" => Some(Timestamp {
                    seconds: 1,
                    nanos: -1,
                }),
                "nanos_overflow" => Some(Timestamp {
                    seconds: 1,
                    nanos: 1_000_000_000,
                }),
                unknown => panic!("unknown created_at vector {unknown}"),
            },
            manifest_sha256: None,
        }
    }

    fn vectors() -> impl Iterator<Item = Vector<'static>> {
        VECTORS.lines().filter_map(|line| {
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let columns = line.split('\t').collect::<Vec<_>>();
            assert_eq!(columns.len(), 6, "invalid shared artifact vector");
            let digest_hex = token(columns[3], "");
            let digest = decode_hex(&digest_hex);
            let digest64 = if digest_hex.len() == 64 {
                digest_hex.as_str()
            } else {
                ""
            };
            let uri = token(columns[2], digest64);
            let artifact_id = if columns[4] == "@matching" {
                format!("sha256:{digest_hex}")
            } else {
                columns[4].to_owned()
            };
            Some(Vector {
                name: columns[0],
                expected: columns[1],
                uri,
                digest,
                artifact_id,
                created_at: columns[5],
            })
        })
    }

    fn token(value: &str, digest: &str) -> String {
        value
            .replace("@digest", digest)
            .replace("@zero32", &"00".repeat(32))
            .replace("@zero31", &"00".repeat(31))
            .replace("@one32", &"11".repeat(32))
            .replace("@upperdigest", &"AA".repeat(32))
            .replace(
                "@overlong",
                &format!("artifact://sha256/{}", "0".repeat(2_100)),
            )
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0);
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).expect("fixture hex is ASCII");
                u8::from_str_radix(text, 16).expect("fixture hex is valid")
            })
            .collect()
    }
}
