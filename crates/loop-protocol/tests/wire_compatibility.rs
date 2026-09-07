use std::fs;
use std::path::PathBuf;

use loop_protocol::wire::v1::{ProtocolInfo, ProtocolLimits, Sha256Digest};
use prost::Message;

const SUPPORTED_PACKAGES: &[&str] = &[
    "loop.audit.v1",
    "loop.discovery.v1",
    "loop.holdout.v1",
    "loop.jobs.v1",
    "loop.protocol.v1",
    "loop.provider.v1",
    "loop.research.v1",
    "loop.v1",
];
const FEATURES: &[&str] = &["artifacts.by-reference.v1", "factors.canonical-json.v1"];
const PRODUCER_FIXTURES: &[&str] = &[
    "protocol_info_v1.binpb",
    "protocol_info_v1.rust.binpb",
    "protocol_info_v1.typescript.binpb",
];

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/contracts/protocol/v1")
        .join(name);
    fs::read(path).expect("committed wire fixture must be readable")
}

fn assert_expected_projection(message: &ProtocolInfo) {
    assert_eq!(
        message.supported_packages,
        SUPPORTED_PACKAGES
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        message.features,
        FEATURES
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        message.limits,
        Some(ProtocolLimits {
            maximum_unary_bytes: 4_194_304,
            maximum_stream_event_bytes: 1_048_576,
            maximum_canonical_ast_bytes: 262_144,
            maximum_ast_nodes: 4_096,
            maximum_ast_depth: 64,
            maximum_page_records: 500,
            maximum_identity_bytes: 128,
            maximum_artifact_uri_bytes: 2_048,
        })
    );
    assert_eq!(message.build_version, "0.2.0-alpha.1+wire-fixture.1");
    assert_eq!(
        message.build_sha256,
        Some(Sha256Digest {
            value: (0_u8..32).collect(),
        })
    );
}

fn expected_protocol_info() -> ProtocolInfo {
    ProtocolInfo {
        supported_packages: SUPPORTED_PACKAGES
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        features: FEATURES.iter().map(|value| (*value).to_owned()).collect(),
        limits: Some(ProtocolLimits {
            maximum_unary_bytes: 4_194_304,
            maximum_stream_event_bytes: 1_048_576,
            maximum_canonical_ast_bytes: 262_144,
            maximum_ast_nodes: 4_096,
            maximum_ast_depth: 64,
            maximum_page_records: 500,
            maximum_identity_bytes: 128,
            maximum_artifact_uri_bytes: 2_048,
        }),
        build_version: "0.2.0-alpha.1+wire-fixture.1".to_owned(),
        build_sha256: Some(Sha256Digest {
            value: (0_u8..32).collect(),
        }),
    }
}

fn assert_semantic_round_trip(name: &str) {
    let decoded = ProtocolInfo::decode(fixture(name).as_slice())
        .expect("current binding must decode the compatibility fixture");
    assert_expected_projection(&decoded);

    let locally_encoded = decoded.encode_to_vec();
    let decoded_again = ProtocolInfo::decode(locally_encoded.as_slice())
        .expect("current binding must decode its own encoding");
    assert_expected_projection(&decoded_again);
}

#[test]
fn generated_core_binding_imports() {
    let message = ProtocolInfo {
        supported_packages: Vec::new(),
        features: Vec::new(),
        limits: None,
        build_version: String::new(),
        build_sha256: None,
    };
    assert_eq!(message.encoded_len(), 0);
}

#[cfg(feature = "protocol-service")]
#[test]
fn generated_protocol_service_binding_imports() {
    use loop_protocol::wire::protocol::v1::GetProtocolInfoRequest;

    assert_eq!(GetProtocolInfoRequest {}.encoded_len(), 0);
}

#[test]
fn rust_fixture_is_current_native_encoder_output() {
    assert_eq!(
        fixture("protocol_info_v1.rust.binpb"),
        expected_protocol_info().encode_to_vec()
    );
}

#[test]
fn decodes_and_reencodes_every_producer_fixture() {
    for name in PRODUCER_FIXTURES {
        assert_semantic_round_trip(name);
    }
}

#[test]
fn old_reader_tolerates_additive_unknown_field() {
    assert_semantic_round_trip("protocol_info_v1_unknown_field.binpb");
    // Unknown-field preservation is intentionally not asserted. Lossless
    // forwarding retains the original envelope instead of parse/re-serialize.
}
