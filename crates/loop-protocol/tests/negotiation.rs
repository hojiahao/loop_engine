use loop_protocol::{
    job::protocol_selection_sha256,
    negotiation::{
        ProtocolBuildIdentity, negotiate_protocol_availability,
        validate_protocol_selection_availability,
    },
    wire::v1::{ProtocolInfo, ProtocolLimits, ProtocolSelectionSnapshot, Sha256Digest},
};
use prost_types::Timestamp;

const VECTORS: &str = include_str!("../../../tests/contracts/protocol_negotiation_vectors.tsv");
const REQUIRED_PACKAGE: &str = "loop.research.v1";
const REQUIRED_FEATURES: &[&str] = &["jobs.envelope.v1"];

#[test]
fn shared_protocol_negotiation_matrix_fails_closed() {
    let rows = VECTORS.lines().skip(1).collect::<Vec<_>>();
    assert_eq!(rows.len(), 15, "all shared rows must execute");
    for row in rows {
        let fields = row.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 4, "malformed shared vector row");
        let (name, operation, expected, mutation) = (fields[0], fields[1], fields[2], fields[3]);
        let result = match operation {
            "negotiate" => run_negotiation(mutation),
            "selection" => run_selection_validation(mutation),
            other => panic!("unknown operation {other}"),
        };
        match result {
            Ok(()) => assert_eq!(expected, "accept", "{name}"),
            Err(code) => assert_eq!(code, expected, "{name}"),
        }
    }
}

fn run_negotiation(mutation: &str) -> Result<(), &'static str> {
    let mut local = protocol_info("client.1", 0x11, true);
    let mut peer = protocol_info("server.1", 0x22, false);
    match mutation {
        "none" => {}
        "local_remove_package" => {
            local.supported_packages.remove(0);
        }
        "peer_remove_package" => {
            peer.supported_packages.remove(0);
        }
        "local_remove_required_feature" => {
            local.features.retain(|value| value != REQUIRED_FEATURES[0])
        }
        "peer_remove_required_feature" => {
            peer.features.retain(|value| value != REQUIRED_FEATURES[0])
        }
        "peer_unsorted_packages" => peer.supported_packages.swap(0, 1),
        other => panic!("unknown negotiation mutation {other}"),
    }
    negotiate_protocol_availability(&local, &peer, REQUIRED_PACKAGE, REQUIRED_FEATURES)
        .map(|negotiated| {
            assert_eq!(
                negotiated.enabled_features,
                ["factors.canonical-json.v1", "jobs.envelope.v1"]
            );
            assert_eq!(
                negotiated.effective_limits,
                peer.limits.expect("fixture limits")
            );
        })
        .map_err(|error| error.code.as_str())
}

fn run_selection_validation(mutation: &str) -> Result<(), &'static str> {
    let local = protocol_info("client.1", 0x11, true);
    let mut selection = protocol_selection();
    let retained_builds = [ProtocolBuildIdentity {
        build_version: "server.1".to_owned(),
        build_sha256: [0x22; 32],
    }];
    let available_descriptors = [[0x33; 32]];
    let recompute_digest = match mutation {
        "none" => true,
        "selection_package" => {
            selection.selected_package = "loop.research.v2".to_owned();
            true
        }
        "selection_remove_required_feature" => {
            selection.enabled_features.remove(1);
            true
        }
        "selection_add_unsupported_feature" => {
            selection
                .enabled_features
                .push("streams.sequence.v1".to_owned());
            true
        }
        "selection_limit_exceeds_local" => {
            selection
                .effective_limits
                .as_mut()
                .expect("fixture limits")
                .maximum_unary_bytes = 3 * 1_024 * 1_024;
            true
        }
        "selection_server_build" => {
            selection.server_build_version = "server.2".to_owned();
            true
        }
        "selection_client_build" => {
            selection.client_build_sha256 = Some(digest(0x44));
            true
        }
        "selection_descriptor" => {
            selection.schema_descriptor_sha256 = Some(digest(0x55));
            true
        }
        "selection_digest" => {
            selection
                .selection_sha256
                .as_mut()
                .expect("fixture digest")
                .value[0] ^= 0xff;
            false
        }
        other => panic!("unknown selection mutation {other}"),
    };
    if recompute_digest {
        selection.selection_sha256 = Some(Sha256Digest {
            value: protocol_selection_sha256(&selection)
                .expect("mutated selection remains canonical")
                .to_vec(),
        });
    }
    validate_protocol_selection_availability(
        &selection,
        &local,
        &retained_builds,
        &available_descriptors,
        REQUIRED_PACKAGE,
        REQUIRED_FEATURES,
    )
    .map_err(|error| error.code.as_str())
}

fn protocol_info(build_version: &str, build_byte: u8, local: bool) -> ProtocolInfo {
    ProtocolInfo {
        supported_packages: vec!["loop.research.v1".to_owned(), "loop.v1".to_owned()],
        features: if local {
            vec![
                "artifacts.by-reference.v1".to_owned(),
                "factors.canonical-json.v1".to_owned(),
                "jobs.envelope.v1".to_owned(),
            ]
        } else {
            vec![
                "factors.canonical-json.v1".to_owned(),
                "jobs.envelope.v1".to_owned(),
                "streams.sequence.v1".to_owned(),
            ]
        },
        limits: Some(if local {
            limits(
                2 * 1_024 * 1_024,
                512 * 1_024,
                128 * 1_024,
                2_048,
                32,
                250,
                128,
                1_024,
            )
        } else {
            limits(
                1_024 * 1_024,
                256 * 1_024,
                64 * 1_024,
                1_024,
                16,
                100,
                64,
                512,
            )
        }),
        build_version: build_version.to_owned(),
        build_sha256: Some(digest(build_byte)),
    }
}

fn protocol_selection() -> ProtocolSelectionSnapshot {
    let mut selection = ProtocolSelectionSnapshot {
        selected_package: REQUIRED_PACKAGE.to_owned(),
        enabled_features: vec![
            "factors.canonical-json.v1".to_owned(),
            "jobs.envelope.v1".to_owned(),
        ],
        effective_limits: Some(limits(
            1_024 * 1_024,
            256 * 1_024,
            64 * 1_024,
            1_024,
            16,
            100,
            64,
            512,
        )),
        server_build_version: "server.1".to_owned(),
        server_build_sha256: Some(digest(0x22)),
        schema_descriptor_sha256: Some(digest(0x33)),
        selection_sha256: None,
        selected_at: Some(Timestamp {
            seconds: 2,
            nanos: 0,
        }),
        client_build_version: "client.1".to_owned(),
        client_build_sha256: Some(digest(0x11)),
    };
    selection.selection_sha256 = Some(Sha256Digest {
        value: protocol_selection_sha256(&selection)
            .expect("fixture selection must be canonical")
            .to_vec(),
    });
    selection
}

#[allow(clippy::too_many_arguments)]
const fn limits(
    unary: u64,
    stream: u64,
    ast: u64,
    nodes: u32,
    depth: u32,
    page: u32,
    identity: u32,
    uri: u32,
) -> ProtocolLimits {
    ProtocolLimits {
        maximum_unary_bytes: unary,
        maximum_stream_event_bytes: stream,
        maximum_canonical_ast_bytes: ast,
        maximum_ast_nodes: nodes,
        maximum_ast_depth: depth,
        maximum_page_records: page,
        maximum_identity_bytes: identity,
        maximum_artifact_uri_bytes: uri,
    }
}

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest {
        value: vec![byte; 32],
    }
}
