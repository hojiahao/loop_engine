use super::*;
use serde::Deserialize;

const GOLDEN: &str = include_str!("../../../../tests/contracts/holdout_identity_golden.json");
const NEGATIVE: &str = include_str!("../../../../tests/contracts/holdout_identity_negative.tsv");

#[derive(Deserialize)]
struct Fixture {
    trusted_plan_schema_sha256: String,
    trusted_backtest_schema_sha256: String,
    backtest_artifacts: Vec<ArtifactFixture>,
    periods: Vec<PeriodFixture>,
    plans: Vec<PlanFixture>,
}

#[derive(Deserialize)]
struct ArtifactFixture {
    sha256: String,
    content: String,
}

#[derive(Deserialize)]
struct PeriodFixture {
    name: String,
    canonical_json: String,
    canonical_sha256: String,
    holdout_period_id: String,
}

#[derive(Deserialize)]
struct PlanFixture {
    name: String,
    period: String,
    canonical_json: String,
    plan_sha256: String,
    holdout_evaluation_plan_id: String,
    entry_count: u32,
}

#[test]
fn shared_golden_periods_and_plans_match_exact_bytes() {
    let fixture: Fixture = serde_json::from_str(GOLDEN).unwrap();
    let trusted_backtest = require_digest_text(
        &fixture.trusted_backtest_schema_sha256,
        "trusted_backtest_schema_sha256",
    )
    .unwrap();
    let trusted_plan = require_digest_text(
        &fixture.trusted_plan_schema_sha256,
        "trusted_plan_schema_sha256",
    )
    .unwrap();
    let resolved = fixture
        .backtest_artifacts
        .iter()
        .map(|artifact| {
            (
                artifact.sha256.clone(),
                artifact.content.as_bytes().to_vec(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    for period_fixture in &fixture.periods {
        let period = parse_canonical_holdout_period(period_fixture.canonical_json.as_bytes())
            .unwrap_or_else(|error| panic!("{}: {error}", period_fixture.name));
        assert_eq!(
            period.canonical_bytes,
            period_fixture.canonical_json.as_bytes()
        );
        assert_eq!(
            encode_digest(&period.canonical_period_sha256),
            period_fixture.canonical_sha256
        );
        assert_eq!(period.holdout_period_id, period_fixture.holdout_period_id);

        for plan_fixture in fixture
            .plans
            .iter()
            .filter(|plan| plan.period == period_fixture.name)
        {
            let plan = parse_canonical_holdout_evaluation_plan(
                plan_fixture.canonical_json.as_bytes(),
                &period,
                &trusted_backtest,
                &resolved,
            )
            .unwrap_or_else(|error| panic!("{}: {error}", plan_fixture.name));
            assert_eq!(plan.canonical_bytes, plan_fixture.canonical_json.as_bytes());
            assert_eq!(encode_digest(&plan.plan_sha256), plan_fixture.plan_sha256);
            assert_eq!(
                plan.holdout_evaluation_plan_id,
                plan_fixture.holdout_evaluation_plan_id
            );

            let raw = plan.plan_sha256;
            let reference = HoldoutEvaluationPlanReference {
                holdout_evaluation_plan_id: plan.holdout_evaluation_plan_id.clone(),
                canonical_plan: PlanArtifactReference {
                    artifact_id: encode_digest(&raw),
                    uri: format!("artifact://sha256/{}", &encode_digest(&raw)[7..]),
                    sha256: raw,
                    schema_name: PLAN_ARTIFACT_SCHEMA_NAME.to_owned(),
                    schema_version: 1,
                    schema_sha256: trusted_plan,
                    media_type: JSON_MEDIA_TYPE.to_owned(),
                    byte_size: plan.canonical_bytes.len() as u64,
                    has_row_count: false,
                    has_manifest_sha256: false,
                },
                plan_sha256: raw,
                entry_count: plan_fixture.entry_count,
                holdout_period_id: period.holdout_period_id.clone(),
                canonical_period_sha256: period.canonical_period_sha256,
            };
            validate_holdout_evaluation_plan_reference(
                &reference,
                &plan.canonical_bytes,
                &period,
                &trusted_plan,
                &trusted_backtest,
                &resolved,
            )
            .unwrap();
            assert_ne!(encode_digest(&raw), plan.holdout_evaluation_plan_id);
        }
    }
}

#[test]
fn strict_period_parser_rejects_aliases_and_noncanonical_json() {
    let fixture: Fixture = serde_json::from_str(GOLDEN).unwrap();
    let source = &fixture.periods[0].canonical_json;
    for invalid in [
        format!(" {source}"),
        source.replacen("{\"schema\":", "{\"unknown\":\"x\",\"schema\":", 1),
        source.replacen(
            "{\"schema\":\"loop.holdout-period/v1\"",
            "{\"schema\":\"loop.holdout-period/v1\",\"schema\":\"loop.holdout-period/v1\"",
            1,
        ),
        source.replace("2024-12-31", "2024-02-30"),
        source.replace("first_locked_confirmation", "development_validation"),
        source.replace(
            "sha256:0101010101010101010101010101010101010101010101010101010101010101\",\"sha256:0202",
            "sha256:0202020202020202020202020202020202020202020202020202020202020202\",\"sha256:0101",
        ),
    ] {
        assert!(parse_canonical_holdout_period(invalid.as_bytes()).is_err());
    }
    let period = parse_canonical_holdout_period(source.as_bytes()).unwrap();
    let wrong = [9_u8; 32];
    assert!(
        verify_holdout_period_identity(source.as_bytes(), &period.holdout_period_id, &wrong)
            .is_err()
    );
}

#[test]
fn strict_plan_parser_rejects_semantic_mutations_and_recursion() {
    let fixture: Fixture = serde_json::from_str(GOLDEN).unwrap();
    let period =
        parse_canonical_holdout_period(fixture.periods[0].canonical_json.as_bytes()).unwrap();
    let trusted = require_digest_text(
        &fixture.trusted_backtest_schema_sha256,
        "trusted_backtest_schema_sha256",
    )
    .unwrap();
    let resolved = fixture
        .backtest_artifacts
        .iter()
        .map(|artifact| {
            (
                artifact.sha256.clone(),
                artifact.content.as_bytes().to_vec(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let source = &fixture.plans[0].canonical_json;
    for invalid in [
        format!(" {source}"),
        source.replace("\"entry_index\":\"2\"", "\"entry_index\":\"3\""),
        source.replace("\"maximum_steps\":\"40\"", "\"maximum_steps\":\"0\""),
        source.replace("\"currency_code\":\"USD\"", "\"currency_code\":\"usd\""),
        source.replace("loop.backtest_spec", "loop.other_spec"),
        source.replace(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
    ] {
        assert!(
            parse_canonical_holdout_evaluation_plan(
                invalid.as_bytes(),
                &period,
                &trusted,
                &resolved
            )
            .is_err()
        );
    }
    let deep = format!("{}0{}", "[".repeat(40), "]".repeat(40));
    assert_eq!(
        parse_canonical_holdout_evaluation_plan(deep.as_bytes(), &period, &trusted, &resolved,)
            .unwrap_err()
            .code,
        HoldoutValidationCode::SizeLimit
    );
}

#[test]
fn every_shared_negative_vector_fails_closed() {
    let fixture: Fixture = serde_json::from_str(GOLDEN).unwrap();
    let period_source = &fixture.periods[0].canonical_json;
    let plan_source = &fixture.plans[0].canonical_json;
    let period = parse_canonical_holdout_period(period_source.as_bytes()).unwrap();
    let trusted_plan = require_digest_text(
        &fixture.trusted_plan_schema_sha256,
        "trusted_plan_schema_sha256",
    )
    .unwrap();
    let trusted_backtest = require_digest_text(
        &fixture.trusted_backtest_schema_sha256,
        "trusted_backtest_schema_sha256",
    )
    .unwrap();
    let resolved = resolved_artifacts(&fixture);
    let plan = parse_canonical_holdout_evaluation_plan(
        plan_source.as_bytes(),
        &period,
        &trusted_backtest,
        &resolved,
    )
    .unwrap();

    for line in NEGATIVE.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns = line.split('\t').collect::<Vec<_>>();
        assert_eq!(columns.len(), 3, "invalid shared holdout vector: {line}");
        let result = execute_negative(
            columns[1],
            columns[2],
            period_source,
            plan_source,
            &period,
            &plan,
            &trusted_plan,
            &trusted_backtest,
            &resolved,
        );
        assert!(
            result.is_err(),
            "negative vector {} was accepted",
            columns[0]
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_negative(
    target: &str,
    mutation: &str,
    period_source: &str,
    plan_source: &str,
    period: &CanonicalHoldoutPeriod,
    plan: &CanonicalHoldoutEvaluationPlan,
    trusted_plan: &[u8; 32],
    trusted_backtest: &[u8; 32],
    resolved: &BTreeMap<String, Vec<u8>>,
) -> Result<(), HoldoutValidationError> {
    match target {
        "period" => {
            parse_canonical_holdout_period(mutate_period(period_source, mutation).as_bytes())
                .map(|_| ())
        }
        "period_reference" => {
            let wrong = [238_u8; 32];
            let wrong_id = digest_text(238);
            verify_holdout_period_identity(
                period_source.as_bytes(),
                if mutation == "period_id_mismatch" {
                    &wrong_id
                } else {
                    &period.holdout_period_id
                },
                if mutation == "period_digest_mismatch" {
                    &wrong
                } else {
                    &period.canonical_period_sha256
                },
            )
            .map(|_| ())
        }
        "plan" => {
            let (source, trusted, artifacts) =
                mutate_plan(plan_source, mutation, trusted_backtest, resolved);
            parse_canonical_holdout_evaluation_plan(
                source.as_bytes(),
                period,
                trusted.as_ref().unwrap_or(trusted_backtest),
                artifacts.as_ref().unwrap_or(resolved),
            )
            .map(|_| ())
        }
        "plan_reference" => {
            let (reference, trusted) = mutate_reference(
                plan_reference(plan, period, plan.value.entries.len() as u32, trusted_plan),
                mutation,
            );
            validate_holdout_evaluation_plan_reference(
                &reference,
                &plan.canonical_bytes,
                period,
                trusted.as_ref().unwrap_or(trusted_plan),
                trusted_backtest,
                resolved,
            )
            .map(|_| ())
        }
        _ => panic!("unimplemented negative target {target}"),
    }
}

fn mutate_period(source: &str, mutation: &str) -> String {
    let mut parsed: serde_json::Value = serde_json::from_str(source).unwrap();
    match mutation {
        "unknown_field" => source.replacen('{', "{\"unknown\":\"x\",", 1),
        "duplicate_schema" => source.replacen(
            "\"schema\":\"loop.holdout-period/v1\"",
            "\"schema\":\"loop.holdout-period/v1\",\"schema\":\"loop.holdout-period/v1\"",
            1,
        ),
        "reorder_top_level" => format!(
            "{{\"sample\":{},\"schema\":{},\"snapshot_ids\":{},\"snapshot_manifest_sha256\":{}}}",
            parsed["sample"],
            parsed["schema"],
            parsed["snapshot_ids"],
            parsed["snapshot_manifest_sha256"]
        ),
        "leading_whitespace" => format!(" {source}"),
        "wrong_schema" => source.replacen("loop.holdout-period/v1", "loop.holdout-period/v2", 1),
        "forbidden_role" => {
            source.replacen("first_locked_confirmation", "development_validation", 1)
        }
        "invalid_date" => source.replacen("2024-12-31", "2024-02-30", 1),
        "reversed_window" => source.replacen("2021-01-01", "2025-01-01", 1),
        "empty_snapshots" => {
            parsed["snapshot_ids"] = serde_json::json!([]);
            parsed.to_string()
        }
        "unsorted_snapshots" => {
            parsed["snapshot_ids"].as_array_mut().unwrap().reverse();
            parsed.to_string()
        }
        "duplicate_snapshots" => {
            let first = parsed["snapshot_ids"][0].clone();
            parsed["snapshot_ids"] = serde_json::json!([first.clone(), first]);
            parsed.to_string()
        }
        "bad_snapshot_digest" => {
            parsed["snapshot_ids"][0] = serde_json::json!(format!("sha256:{}", "A".repeat(64)));
            parsed.to_string()
        }
        "bad_manifest_digest" => {
            parsed["snapshot_manifest_sha256"] =
                serde_json::json!(format!("sha256:{}", "g".repeat(64)));
            parsed.to_string()
        }
        "number_date" => {
            parsed["sample"]["start_inclusive"] = serde_json::json!(20_210_101);
            parsed.to_string()
        }
        "deep_nesting" => format!("{}0{}", "[".repeat(40), "]".repeat(40)),
        _ => panic!("unimplemented period mutation {mutation}"),
    }
}

type PlanMutation = (String, Option<[u8; 32]>, Option<BTreeMap<String, Vec<u8>>>);

fn mutate_plan(
    source: &str,
    mutation: &str,
    _trusted_backtest: &[u8; 32],
    resolved: &BTreeMap<String, Vec<u8>>,
) -> PlanMutation {
    let mut parsed: serde_json::Value = serde_json::from_str(source).unwrap();
    match mutation {
        "unknown_field" => parsed["unknown"] = serde_json::json!("x"),
        "duplicate_schema" => {
            return (
                source.replacen(
                    "\"schema\":\"loop.holdout-evaluation-plan/v1\"",
                    "\"schema\":\"loop.holdout-evaluation-plan/v1\",\"schema\":\"loop.holdout-evaluation-plan/v1\"",
                    1,
                ),
                None,
                None,
            );
        }
        "reorder_top_level" => {
            return (
                format!(
                    "{{\"holdout_period_id\":{},\"schema\":{},\"canonical_period_sha256\":{},\"entries\":{}}}",
                    parsed["holdout_period_id"],
                    parsed["schema"],
                    parsed["canonical_period_sha256"],
                    parsed["entries"]
                ),
                None,
                None,
            );
        }
        "leading_whitespace" => return (format!(" {source}"), None, None),
        "wrong_schema" => {
            parsed["schema"] = serde_json::json!("loop.holdout-evaluation-plan/v2");
        }
        "period_id_mismatch" => {
            parsed["holdout_period_id"] = serde_json::json!(digest_text(225));
        }
        "period_digest_mismatch" => {
            parsed["canonical_period_sha256"] = serde_json::json!(digest_text(226));
        }
        "empty_entries" => parsed["entries"] = serde_json::json!([]),
        "noncontiguous_entries" => parsed["entries"][1]["entry_index"] = serde_json::json!("3"),
        "duplicate_factor" => {
            parsed["entries"][1]["factor_spec_id"] = parsed["entries"][0]["factor_spec_id"].clone();
        }
        "duplicate_backtest" => {
            parsed["entries"][1]["backtest_spec_artifact"] =
                parsed["entries"][0]["backtest_spec_artifact"].clone();
        }
        "inline_backtest" => {
            parsed["entries"][0]["backtest_spec_artifact"]["inline"] =
                serde_json::json!({"schema": "forbidden"});
        }
        "bad_locator" => {
            parsed["entries"][0]["backtest_spec_artifact"]["uri"] =
                serde_json::json!("https://user:secret@example.invalid/value");
        }
        "wrong_artifact_schema" => {
            parsed["entries"][0]["backtest_spec_artifact"]["schema_name"] =
                serde_json::json!("loop.other_spec");
        }
        "wrong_artifact_version" => {
            parsed["entries"][0]["backtest_spec_artifact"]["schema_version"] =
                serde_json::json!("2");
        }
        "wrong_backtest_schema_digest" => {
            parsed["entries"][0]["backtest_spec_artifact"]["schema_sha256"] =
                serde_json::json!(digest_text(227));
        }
        "wrong_media_type" => {
            parsed["entries"][0]["backtest_spec_artifact"]["media_type"] =
                serde_json::json!("application/octet-stream");
        }
        "zero_artifact_size" => {
            parsed["entries"][0]["backtest_spec_artifact"]["byte_size"] = serde_json::json!("0");
        }
        "oversized_artifact" => {
            parsed["entries"][0]["backtest_spec_artifact"]["byte_size"] =
                serde_json::json!("268435457");
        }
        "non_normalized_artifact_size" => {
            parsed["entries"][0]["backtest_spec_artifact"]["byte_size"] = serde_json::json!("050");
        }
        "zero_steps" => {
            parsed["entries"][0]["job_budget"]["maximum_steps"] = serde_json::json!("0");
        }
        "non_normalized_steps" => {
            parsed["entries"][0]["job_budget"]["maximum_steps"] = serde_json::json!("040");
        }
        "token_overflow" => {
            parsed["entries"][0]["job_budget"]["maximum_input_tokens"] =
                serde_json::json!("1000000000001");
        }
        "negative_token" => {
            parsed["entries"][0]["job_budget"]["maximum_output_tokens"] = serde_json::json!("-1");
        }
        "cost_overflow" => {
            parsed["entries"][0]["job_budget"]["maximum_cost"]["amount"] =
                serde_json::json!("1000000.1");
        }
        "cost_precision" => {
            parsed["entries"][0]["job_budget"]["maximum_cost"]["amount"] =
                serde_json::json!("1234567890123456789");
        }
        "cost_scale" => {
            parsed["entries"][0]["job_budget"]["maximum_cost"]["amount"] =
                serde_json::json!("0.1234567891");
        }
        "lowercase_currency" => {
            parsed["entries"][0]["job_budget"]["maximum_cost"]["currency_code"] =
                serde_json::json!("usd");
        }
        "zero_wall_time" => {
            parsed["entries"][0]["job_budget"]["maximum_wall_time_ns"] = serde_json::json!("0");
        }
        "wall_time_overflow" => {
            parsed["entries"][0]["job_budget"]["maximum_wall_time_ns"] =
                serde_json::json!("604800000000001");
        }
        "unresolved_artifact" => {
            let digest = parsed["entries"][0]["backtest_spec_artifact"]["sha256"]
                .as_str()
                .unwrap();
            let mut reduced = resolved.clone();
            reduced.remove(digest);
            return (source.to_owned(), None, Some(reduced));
        }
        "artifact_content_mismatch" => {
            let digest = parsed["entries"][0]["backtest_spec_artifact"]["sha256"]
                .as_str()
                .unwrap();
            let mut changed = resolved.clone();
            changed.insert(digest.to_owned(), b"different bytes".to_vec());
            return (source.to_owned(), None, Some(changed));
        }
        "deep_nesting" => {
            return (format!("{}0{}", "[".repeat(40), "]".repeat(40)), None, None);
        }
        _ => panic!("unimplemented plan mutation {mutation}"),
    }
    (parsed.to_string(), None, None)
}

fn mutate_reference(
    mut reference: HoldoutEvaluationPlanReference,
    mutation: &str,
) -> (HoldoutEvaluationPlanReference, Option<[u8; 32]>) {
    match mutation {
        "plan_sha256_mismatch" => reference.plan_sha256 = [230_u8; 32],
        "plan_id_mismatch" => reference.holdout_evaluation_plan_id = digest_text(231),
        "entry_count_mismatch" => reference.entry_count += 1,
        "period_id_mismatch" => reference.holdout_period_id = digest_text(232),
        "period_digest_mismatch" => reference.canonical_period_sha256 = [233_u8; 32],
        "plan_artifact_schema_mismatch" => {
            reference.canonical_plan.schema_name = "loop.other_plan".to_owned();
        }
        "plan_schema_digest_mismatch" => {
            reference.canonical_plan.schema_sha256 = [234_u8; 32];
            return (reference, Some([239_u8; 32]));
        }
        "plan_artifact_size_mismatch" => reference.canonical_plan.byte_size += 1,
        "plan_artifact_locator_mismatch" => {
            reference.canonical_plan.uri = format!("artifact://sha256/{}", "00".repeat(32));
        }
        _ => panic!("unimplemented plan reference mutation {mutation}"),
    }
    (reference, None)
}

fn resolved_artifacts(fixture: &Fixture) -> BTreeMap<String, Vec<u8>> {
    fixture
        .backtest_artifacts
        .iter()
        .map(|artifact| {
            (
                artifact.sha256.clone(),
                artifact.content.as_bytes().to_vec(),
            )
        })
        .collect()
}

fn plan_reference(
    plan: &CanonicalHoldoutEvaluationPlan,
    period: &CanonicalHoldoutPeriod,
    entry_count: u32,
    trusted_plan: &[u8; 32],
) -> HoldoutEvaluationPlanReference {
    let raw = encode_digest(&plan.plan_sha256);
    HoldoutEvaluationPlanReference {
        holdout_evaluation_plan_id: plan.holdout_evaluation_plan_id.clone(),
        canonical_plan: PlanArtifactReference {
            artifact_id: raw.clone(),
            uri: format!("artifact://sha256/{}", &raw[7..]),
            sha256: plan.plan_sha256,
            schema_name: PLAN_ARTIFACT_SCHEMA_NAME.to_owned(),
            schema_version: 1,
            schema_sha256: *trusted_plan,
            media_type: JSON_MEDIA_TYPE.to_owned(),
            byte_size: plan.canonical_bytes.len() as u64,
            has_row_count: false,
            has_manifest_sha256: false,
        },
        plan_sha256: plan.plan_sha256,
        entry_count,
        holdout_period_id: period.holdout_period_id.clone(),
        canonical_period_sha256: period.canonical_period_sha256,
    }
}

fn digest_text(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}
