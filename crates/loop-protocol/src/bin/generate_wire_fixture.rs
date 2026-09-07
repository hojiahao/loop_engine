use std::{env, fs, process};

use loop_protocol::wire::v1::{ProtocolInfo, ProtocolLimits, Sha256Digest};
use prost::Message;

fn protocol_info() -> ProtocolInfo {
    ProtocolInfo {
        supported_packages: [
            "loop.audit.v1",
            "loop.discovery.v1",
            "loop.holdout.v1",
            "loop.jobs.v1",
            "loop.protocol.v1",
            "loop.provider.v1",
            "loop.research.v1",
            "loop.v1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        features: ["artifacts.by-reference.v1", "factors.canonical-json.v1"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
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

fn main() {
    let mut arguments = env::args_os().skip(1);
    let Some(output) = arguments.next() else {
        eprintln!("usage: generate_wire_fixture <output-file>");
        process::exit(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: generate_wire_fixture <output-file>");
        process::exit(2);
    }
    if let Err(error) = fs::write(output, protocol_info().encode_to_vec()) {
        eprintln!("failed to write Rust wire fixture: {error}");
        process::exit(1);
    }
}
