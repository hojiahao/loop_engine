use clap::Parser;

use super::*;

fn cli(args: &[&str]) -> Result<crate::Cli, clap::Error> {
    crate::Cli::try_parse_from(
        ["loopctl", "discovery", "--config", "/tmp/client.json"]
            .into_iter()
            .chain(args.iter().copied()),
    )
}

fn time() -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: 1_800_000_000,
        nanos: 0,
    }
}

fn money() -> v1::Money {
    v1::Money {
        amount: Some(v1::ExactDecimal { value: "1".into() }),
        currency_code: "USD".into(),
    }
}

fn digest() -> v1::Sha256Digest {
    v1::Sha256Digest { value: vec![7; 32] }
}

fn input() -> wire::DiscoveryJobInput {
    let model = v1::ModelResolutionSnapshot {
        resolution_id: Some(v1::ModelResolutionId {
            value: "resolution.fixture".into(),
        }),
        provider_id: Some(v1::ProviderId {
            value: "provider.fixture".into(),
        }),
        model_id: Some(v1::ModelId {
            value: "model.fixture".into(),
        }),
        requested_alias: "fixture-model".into(),
        protocol_family: v1::ModelProtocolFamily::OpenaiResponses as i32,
        capabilities: Some(v1::ModelCapabilities {
            context_window_tokens: 10_000,
            maximum_output_tokens: 1_000,
            ..Default::default()
        }),
        pricing: Some(v1::ModelPricing {
            input_per_million_tokens: Some(money()),
            output_per_million_tokens: Some(money()),
            cached_input_per_million_tokens: Some(money()),
            cache_creation_per_million_tokens: None,
        }),
        capability_sha256: Some(digest()),
        catalog_sha256: Some(digest()),
        resolved_at: Some(time()),
        provider_plugin_name: "fixture-provider".into(),
        provider_plugin_version: "1.0.0".into(),
        provider_plugin_sha256: Some(digest()),
        snapshot_sha256: Some(digest()),
    };
    wire::DiscoveryJobInput {
        dataset: Some(v1::DevelopmentDatasetReference {
            snapshot_ids: vec![v1::SnapshotId {
                value: "snapshot.development".into(),
            }],
            manifest_sha256: Some(digest()),
        }),
        research_policy: Some(v1::PolicyReference {
            policy_id: Some(v1::PolicyId {
                value: "policy.discovery".into(),
            }),
            revision: "1".into(),
            sha256: Some(digest()),
        }),
        maker_model: Some(model.clone()),
        checker_model: Some(model),
        budget: Some(wire::DiscoveryJobBudget {
            maximum_steps: 1,
            maximum_input_tokens: 1_000,
            maximum_output_tokens: 1_000,
            maximum_cost: Some(money()),
            maximum_wall_time: Some(prost_types::Duration {
                seconds: 30,
                nanos: 0,
            }),
        }),
        maximum_candidates: 1,
    }
}

#[test]
fn requires_mutation_key() {
    assert!(cli(&["start", "--input", "/tmp/input.pb"]).is_err());
}

#[test]
fn requires_mutation_revision() {
    for command in [
        "execute",
        "pause",
        "cancel",
        "resume",
        "expire",
        "reconcile",
    ] {
        assert!(cli(&[command, "--job", "job.fixture", "--key", "key.fixture"]).is_err());
    }
}

#[test]
fn parses_lifecycle_commands() {
    for command in [
        "execute",
        "pause",
        "cancel",
        "resume",
        "expire",
        "reconcile",
    ] {
        assert!(
            cli(&[
                command,
                "--job",
                "job.fixture",
                "--revision",
                "1",
                "--key",
                "key.fixture"
            ])
            .is_ok()
        );
    }
}

#[test]
fn bounds_deadline() {
    for timeout in ["0", "121"] {
        assert!(
            cli(&[
                "--timeout-seconds",
                timeout,
                "status",
                "--job",
                "job.fixture"
            ])
            .is_err()
        );
    }
}

#[test]
fn bounds_event_limit() {
    for limit in ["0", "101"] {
        assert!(cli(&["events", "--job", "job.fixture", "--limit", limit]).is_err());
    }
}

#[test]
fn event_defaults() {
    let crate::Command::Discovery(arguments) =
        cli(&["events", "--job", "job.fixture"]).unwrap().command
    else {
        panic!("expected Discovery")
    };
    assert_eq!(arguments.timeout_seconds, 30);
    assert!(matches!(
        arguments.command,
        Command::Events {
            after: 0,
            limit: 100,
            ..
        }
    ));
}

#[test]
fn stable_command_identity() {
    let key = "k".repeat(128);
    let left = context(v1::Actor::default(), key.clone()).unwrap();
    let right = context(v1::Actor::default(), key).unwrap();
    assert_ne!(left.request_id, right.request_id);
    assert_eq!(left.correlation_id, right.correlation_id);
    assert_eq!(left.causation_id, right.causation_id);
    assert!(valid_id(&left.correlation_id.unwrap().value));
    assert!(valid_id(&left.causation_id.unwrap().value));
}

#[test]
fn separates_command_keys() {
    let left = context(v1::Actor::default(), "key.one".into()).unwrap();
    let right = context(v1::Actor::default(), "key.two".into()).unwrap();
    assert_ne!(left.correlation_id, right.correlation_id);
    assert_ne!(left.causation_id, right.causation_id);
}

#[test]
fn reads_fresh_identity() {
    let command = Command::Status {
        job: "job.fixture".into(),
    };
    assert_ne!(command.key(), command.key());
}

#[test]
fn accepts_canonical_input() {
    let input = input();
    assert_eq!(
        decode_input(&input.encode_to_vec(), &time()).unwrap(),
        input
    );
}

#[test]
fn rejects_unknown_wire() {
    let mut bytes = input().encode_to_vec();
    bytes.extend_from_slice(&[0x38, 1]);
    assert_eq!(decode_input(&bytes, &time()), Err(Failure::Input));
}

#[test]
fn rejects_duplicate_wire() {
    let mut bytes = input().encode_to_vec();
    bytes.extend_from_slice(&[0x30, 1]);
    assert_eq!(decode_input(&bytes, &time()), Err(Failure::Input));
}

#[test]
fn rejects_nonminimal_wire() {
    let mut bytes = input().encode_to_vec();
    assert_eq!(bytes.pop(), Some(1));
    bytes.extend_from_slice(&[0x81, 0]);
    assert_eq!(decode_input(&bytes, &time()), Err(Failure::Input));
}

#[test]
fn rejects_missing_dataset() {
    let mut input = input();
    input.dataset = None;
    assert_eq!(
        decode_input(&input.encode_to_vec(), &time()),
        Err(Failure::Input)
    );
}

#[test]
fn rejects_future_resolution() {
    let mut input = input();
    input
        .maker_model
        .as_mut()
        .unwrap()
        .resolved_at
        .as_mut()
        .unwrap()
        .seconds += 1;
    assert_eq!(
        decode_input(&input.encode_to_vec(), &time()),
        Err(Failure::Input)
    );
}
