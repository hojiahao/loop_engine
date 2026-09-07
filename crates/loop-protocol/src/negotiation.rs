//! Pure protocol availability and negotiation checks for preflight gates.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::job::protocol_selection_sha256;
use crate::wire::v1::{ProtocolInfo, ProtocolLimits, ProtocolSelectionSnapshot};

const MAX_PACKAGES: usize = 16;
const MAX_FEATURES: usize = 256;
const MAX_NAME_BYTES: usize = 128;
const MAX_BUILD_VERSION_BYTES: usize = 128;
const MAX_UNARY_BYTES: u64 = 4 * 1_024 * 1_024;
const MAX_STREAM_EVENT_BYTES: u64 = 1_024 * 1_024;
const MAX_CANONICAL_AST_BYTES: u64 = 256 * 1_024;
const MAX_AST_NODES: u32 = 4_096;
const MAX_AST_DEPTH: u32 = 64;
const MAX_PAGE_RECORDS: u32 = 500;
const MAX_IDENTITY_BYTES: u32 = 128;
const MAX_ARTIFACT_URI_BYTES: u32 = 2_048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolNegotiationCode {
    InvalidProtocolInfo,
    InvalidRequirement,
    UnsupportedMajor,
    UnsupportedRequiredFeature,
    UnsupportedFeature,
    IncompatibleLimits,
    InvalidSelection,
    UnavailableBuild,
    UnavailableDescriptor,
}

impl ProtocolNegotiationCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidProtocolInfo => "invalid_protocol_info",
            Self::InvalidRequirement => "invalid_requirement",
            Self::UnsupportedMajor => "unsupported_major",
            Self::UnsupportedRequiredFeature => "unsupported_required_feature",
            Self::UnsupportedFeature => "unsupported_feature",
            Self::IncompatibleLimits => "incompatible_limits",
            Self::InvalidSelection => "invalid_selection",
            Self::UnavailableBuild => "unavailable_build",
            Self::UnavailableDescriptor => "unavailable_descriptor",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolNegotiationError {
    pub code: ProtocolNegotiationCode,
}

impl ProtocolNegotiationError {
    const fn new(code: ProtocolNegotiationCode) -> Self {
        Self { code }
    }
}

impl Display for ProtocolNegotiationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.as_str())
    }
}

impl Error for ProtocolNegotiationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolBuildIdentity {
    pub build_version: String,
    pub build_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq)]
pub struct NegotiatedProtocol {
    pub selected_package: String,
    pub enabled_features: Vec<String>,
    pub effective_limits: ProtocolLimits,
    pub local_build: ProtocolBuildIdentity,
    pub peer_build: ProtocolBuildIdentity,
}

/// Validate the closed availability projection advertised by one deployed peer.
pub fn validate_protocol_info(info: &ProtocolInfo) -> Result<(), ProtocolNegotiationError> {
    if info.supported_packages.is_empty()
        || info.supported_packages.len() > MAX_PACKAGES
        || !is_sorted_unique(&info.supported_packages)
        || info
            .supported_packages
            .iter()
            .any(|package| !is_protocol_package(package))
        || info.features.len() > MAX_FEATURES
        || !is_sorted_unique(&info.features)
        || info
            .features
            .iter()
            .any(|feature| !is_protocol_feature(feature))
        || !is_valid_limits(info.limits.as_ref())
        || !is_build_version(&info.build_version)
        || info
            .build_sha256
            .as_ref()
            .is_none_or(|digest| digest.value.len() != 32)
    {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::InvalidProtocolInfo,
        ));
    }
    Ok(())
}

/// Negotiate one explicitly required service major before mutable or paid work.
///
/// Features are the complete sorted intersection. Required features must be in
/// that intersection; limits are reduced independently to the stricter peer.
pub fn negotiate_protocol_availability(
    local: &ProtocolInfo,
    peer: &ProtocolInfo,
    required_package: &str,
    required_features: &[&str],
) -> Result<NegotiatedProtocol, ProtocolNegotiationError> {
    validate_protocol_info(local)?;
    validate_protocol_info(peer)?;
    validate_requirement(required_package, required_features)?;
    if local
        .supported_packages
        .binary_search_by(|value| value.as_str().cmp(required_package))
        .is_err()
        || peer
            .supported_packages
            .binary_search_by(|value| value.as_str().cmp(required_package))
            .is_err()
    {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::UnsupportedMajor,
        ));
    }

    let enabled_features = sorted_intersection(&local.features, &peer.features);
    if required_features.iter().any(|required| {
        enabled_features
            .binary_search_by(|value| value.as_str().cmp(required))
            .is_err()
    }) {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::UnsupportedRequiredFeature,
        ));
    }

    Ok(NegotiatedProtocol {
        selected_package: required_package.to_owned(),
        enabled_features,
        effective_limits: minimum_limits(
            local.limits.as_ref().expect("validated local limits"),
            peer.limits.as_ref().expect("validated peer limits"),
        ),
        local_build: protocol_info_build(local),
        peer_build: protocol_info_build(peer),
    })
}

/// Prove a pinned selection remains executable from local immutable content.
///
/// The current local build is always considered available. Both peer builds
/// and the schema descriptor must otherwise occur in the retained stores.
pub fn validate_protocol_selection_availability(
    selection: &ProtocolSelectionSnapshot,
    local: &ProtocolInfo,
    retained_builds: &[ProtocolBuildIdentity],
    available_schema_descriptors: &[[u8; 32]],
    required_package: &str,
    required_features: &[&str],
) -> Result<(), ProtocolNegotiationError> {
    validate_protocol_info(local)?;
    validate_requirement(required_package, required_features)?;
    let computed = protocol_selection_sha256(selection)
        .map_err(|_| ProtocolNegotiationError::new(ProtocolNegotiationCode::InvalidSelection))?;
    let claimed = selection
        .selection_sha256
        .as_ref()
        .ok_or_else(|| ProtocolNegotiationError::new(ProtocolNegotiationCode::InvalidSelection))?;
    if claimed.value.len() != 32 || !constant_time_eq(&claimed.value, &computed) {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::InvalidSelection,
        ));
    }
    if selection.selected_package != required_package
        || local
            .supported_packages
            .binary_search(&selection.selected_package)
            .is_err()
    {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::UnsupportedMajor,
        ));
    }
    if required_features.iter().any(|required| {
        selection
            .enabled_features
            .binary_search_by(|value| value.as_str().cmp(required))
            .is_err()
    }) {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::UnsupportedRequiredFeature,
        ));
    }
    if selection.enabled_features.iter().any(|feature| {
        local
            .features
            .binary_search_by(|value| value.as_str().cmp(feature))
            .is_err()
    }) {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::UnsupportedFeature,
        ));
    }
    if !limits_fit(
        selection
            .effective_limits
            .as_ref()
            .expect("canonical selection validation requires limits"),
        local.limits.as_ref().expect("validated local limits"),
    ) {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::IncompatibleLimits,
        ));
    }

    let server_build = selection_build(
        &selection.server_build_version,
        selection.server_build_sha256.as_ref(),
    )?;
    let client_build = selection_build(
        &selection.client_build_version,
        selection.client_build_sha256.as_ref(),
    )?;
    if !build_is_available(&server_build, local, retained_builds)
        || !build_is_available(&client_build, local, retained_builds)
    {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::UnavailableBuild,
        ));
    }

    let descriptor = selection
        .schema_descriptor_sha256
        .as_ref()
        .ok_or_else(|| ProtocolNegotiationError::new(ProtocolNegotiationCode::InvalidSelection))?;
    let descriptor: [u8; 32] =
        descriptor.value.as_slice().try_into().map_err(|_| {
            ProtocolNegotiationError::new(ProtocolNegotiationCode::InvalidSelection)
        })?;
    if !available_schema_descriptors
        .iter()
        .any(|available| constant_time_eq(available, &descriptor))
    {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::UnavailableDescriptor,
        ));
    }
    Ok(())
}

fn validate_requirement(
    required_package: &str,
    required_features: &[&str],
) -> Result<(), ProtocolNegotiationError> {
    if !is_protocol_package(required_package)
        || required_features.len() > MAX_FEATURES
        || required_features
            .iter()
            .any(|feature| !is_protocol_feature(feature))
        || required_features
            .windows(2)
            .any(|window| window[0] >= window[1])
    {
        return Err(ProtocolNegotiationError::new(
            ProtocolNegotiationCode::InvalidRequirement,
        ));
    }
    Ok(())
}

fn protocol_info_build(info: &ProtocolInfo) -> ProtocolBuildIdentity {
    ProtocolBuildIdentity {
        build_version: info.build_version.clone(),
        build_sha256: info
            .build_sha256
            .as_ref()
            .expect("validated build digest")
            .value
            .as_slice()
            .try_into()
            .expect("validated build digest length"),
    }
}

fn selection_build(
    version: &str,
    digest: Option<&crate::wire::v1::Sha256Digest>,
) -> Result<ProtocolBuildIdentity, ProtocolNegotiationError> {
    let bytes: [u8; 32] = digest
        .ok_or_else(|| ProtocolNegotiationError::new(ProtocolNegotiationCode::InvalidSelection))?
        .value
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolNegotiationError::new(ProtocolNegotiationCode::InvalidSelection))?;
    Ok(ProtocolBuildIdentity {
        build_version: version.to_owned(),
        build_sha256: bytes,
    })
}

fn build_is_available(
    requested: &ProtocolBuildIdentity,
    local: &ProtocolInfo,
    retained: &[ProtocolBuildIdentity],
) -> bool {
    let current = protocol_info_build(local);
    build_identity_eq(requested, &current)
        || retained
            .iter()
            .any(|available| build_identity_eq(requested, available))
}

fn build_identity_eq(left: &ProtocolBuildIdentity, right: &ProtocolBuildIdentity) -> bool {
    left.build_version == right.build_version
        && constant_time_eq(&left.build_sha256, &right.build_sha256)
}

fn minimum_limits(left: &ProtocolLimits, right: &ProtocolLimits) -> ProtocolLimits {
    ProtocolLimits {
        maximum_unary_bytes: left.maximum_unary_bytes.min(right.maximum_unary_bytes),
        maximum_stream_event_bytes: left
            .maximum_stream_event_bytes
            .min(right.maximum_stream_event_bytes),
        maximum_canonical_ast_bytes: left
            .maximum_canonical_ast_bytes
            .min(right.maximum_canonical_ast_bytes),
        maximum_ast_nodes: left.maximum_ast_nodes.min(right.maximum_ast_nodes),
        maximum_ast_depth: left.maximum_ast_depth.min(right.maximum_ast_depth),
        maximum_page_records: left.maximum_page_records.min(right.maximum_page_records),
        maximum_identity_bytes: left
            .maximum_identity_bytes
            .min(right.maximum_identity_bytes),
        maximum_artifact_uri_bytes: left
            .maximum_artifact_uri_bytes
            .min(right.maximum_artifact_uri_bytes),
    }
}

fn limits_fit(selected: &ProtocolLimits, local: &ProtocolLimits) -> bool {
    selected.maximum_unary_bytes <= local.maximum_unary_bytes
        && selected.maximum_stream_event_bytes <= local.maximum_stream_event_bytes
        && selected.maximum_canonical_ast_bytes <= local.maximum_canonical_ast_bytes
        && selected.maximum_ast_nodes <= local.maximum_ast_nodes
        && selected.maximum_ast_depth <= local.maximum_ast_depth
        && selected.maximum_page_records <= local.maximum_page_records
        && selected.maximum_identity_bytes <= local.maximum_identity_bytes
        && selected.maximum_artifact_uri_bytes <= local.maximum_artifact_uri_bytes
}

fn is_valid_limits(value: Option<&ProtocolLimits>) -> bool {
    value.is_some_and(|limits| {
        limits.maximum_unary_bytes > 0
            && limits.maximum_unary_bytes <= MAX_UNARY_BYTES
            && limits.maximum_stream_event_bytes > 0
            && limits.maximum_stream_event_bytes <= MAX_STREAM_EVENT_BYTES
            && limits.maximum_canonical_ast_bytes > 0
            && limits.maximum_canonical_ast_bytes <= MAX_CANONICAL_AST_BYTES
            && limits.maximum_ast_nodes > 0
            && limits.maximum_ast_nodes <= MAX_AST_NODES
            && limits.maximum_ast_depth > 0
            && limits.maximum_ast_depth <= MAX_AST_DEPTH
            && limits.maximum_page_records > 0
            && limits.maximum_page_records <= MAX_PAGE_RECORDS
            && limits.maximum_identity_bytes > 0
            && limits.maximum_identity_bytes <= MAX_IDENTITY_BYTES
            && limits.maximum_artifact_uri_bytes > 0
            && limits.maximum_artifact_uri_bytes <= MAX_ARTIFACT_URI_BYTES
            && limits.maximum_canonical_ast_bytes <= limits.maximum_unary_bytes
            && u64::from(limits.maximum_identity_bytes) <= limits.maximum_unary_bytes
            && u64::from(limits.maximum_artifact_uri_bytes) <= limits.maximum_unary_bytes
    })
}

fn sorted_intersection(left: &[String], right: &[String]) -> Vec<String> {
    let (mut left_index, mut right_index) = (0, 0);
    let mut result = Vec::new();
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                result.push(left[left_index].clone());
                left_index += 1;
                right_index += 1;
            }
        }
    }
    result
}

fn is_sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|window| window[0] < window[1])
}

fn is_protocol_package(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_NAME_BYTES || !value.is_ascii() {
        return false;
    }
    let mut count = 0_usize;
    for (index, part) in value.split('.').enumerate() {
        count += 1;
        if index + 1 == value.split('.').count() {
            if !part.strip_prefix('v').is_some_and(|digits| {
                !digits.is_empty()
                    && !digits.starts_with('0')
                    && digits.bytes().all(|byte| byte.is_ascii_digit())
            }) {
                return false;
            }
        } else if !is_lower_identifier_segment(part) {
            return false;
        }
    }
    count >= 2
}

fn is_protocol_feature(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAME_BYTES
        && value.is_ascii()
        && value.split('.').count() >= 2
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && segment
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && segment
                    .bytes()
                    .last()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn is_build_version(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_BUILD_VERSION_BYTES || !value.is_ascii() {
        return false;
    }
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'_' | b'-'))
}

fn is_lower_identifier_segment(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}
