use super::*;
use std::collections::BTreeMap;

fn fixture() -> (
    serde_json::Value,
    wire::BacktestResult,
    BTreeMap<String, String>,
) {
    let summary = serde_json::json!({
        "schema": "loop.statistics-summary/v1", "factor_spec_id": "factor-test",
        "benchmark": "zero-daily-return",
        "sharpe_annualization": "sqrt(252)-descriptive-no-serial-correction",
        "return_basis": "cost-adjusted-simple-nav-return",
        "sharpe_dispersion": "sample-standard-deviation-ddof-1",
        "observed_sessions": 21, "return_observations": 20, "production_eligible": false,
        "policy": {"groups": 2, "minimum_cross_section": 3, "minimum_sessions": 8, "hac_lags": 1, "pbo_blocks": 4},
        "sharpe_annualized": {"status": "available", "value": 1.5, "reason": null, "observations": 20},
    });
    let result = wire::BacktestResult {
        metrics: vec![wire::BacktestMetric {
            name: "sharpe_annualized".to_owned(),
            value: Some(wire::ExactDecimal {
                value: "1.5".to_owned(),
            }),
            unit: "ratio".to_owned(),
            estimator: "daily-statistics.1".to_owned(),
        }],
        ..Default::default()
    };
    let settings = [
        ("groups", "2"),
        ("minimum_cross_section", "3"),
        ("minimum_sessions", "8"),
        ("hac_lags", "1"),
        ("pbo_blocks", "4"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    (summary, result, settings)
}

#[test]
fn actual_sharpe() {
    let (summary, result, settings) = fixture();
    let value = statistic(&summary, &result, "factor-test", &settings).unwrap();
    assert_eq!(value.value, Some(1.5));
    assert_eq!(value.observations, 20);
}

#[test]
fn unavailable_sharpe() {
    let (mut summary, mut result, settings) = fixture();
    summary["sharpe_annualized"] = serde_json::json!({"status":"unavailable", "value":null, "reason":"constant_returns", "observations":20});
    result.metrics.clear();
    let value = statistic(&summary, &result, "factor-test", &settings).unwrap();
    assert_eq!(value.value, None);
    assert_eq!(value.reason.as_deref(), Some("constant_returns"));
}

#[test]
fn missing_metric() {
    let (summary, mut result, settings) = fixture();
    result.metrics.clear();
    assert!(statistic(&summary, &result, "factor-test", &settings).is_err());
}

#[test]
fn zero_fabrication() {
    let (mut summary, mut result, settings) = fixture();
    summary["sharpe_annualized"] = serde_json::json!({"status":"unavailable", "value":null, "reason":"insufficient_complete_returns", "observations":20});
    result.metrics[0].value.as_mut().unwrap().value = "0".to_owned();
    assert!(statistic(&summary, &result, "factor-test", &settings).is_err());
}

#[test]
fn metric_mismatch() {
    let (summary, mut result, settings) = fixture();
    result.metrics[0].value.as_mut().unwrap().value = "1.6".to_owned();
    assert!(statistic(&summary, &result, "factor-test", &settings).is_err());
}

#[test]
fn legacy_label() {
    let (summary, mut result, settings) = fixture();
    result.metrics[0].name = "net_sharpe".to_owned();
    assert!(statistic(&summary, &result, "factor-test", &settings).is_err());
}

#[test]
fn semantic_mismatch() {
    let (summary, result, settings) = fixture();
    for key in [
        "benchmark",
        "sharpe_annualization",
        "return_basis",
        "sharpe_dispersion",
    ] {
        let mut changed = summary.clone();
        changed[key] = serde_json::json!("different");
        assert!(
            statistic(&changed, &result, "factor-test", &settings).is_err(),
            "{key}"
        );
    }
}

#[test]
fn policy_mismatch() {
    let (mut summary, result, settings) = fixture();
    summary["policy"]["minimum_sessions"] = serde_json::json!(16);
    assert!(statistic(&summary, &result, "factor-test", &settings).is_err());
}

#[test]
fn duplicate_metric() {
    let (summary, mut result, settings) = fixture();
    result.metrics.push(result.metrics[0].clone());
    assert!(statistic(&summary, &result, "factor-test", &settings).is_err());
}

#[test]
fn extreme_statistic() {
    let (mut summary, mut result, settings) = fixture();
    summary["sharpe_annualized"]["value"] = serde_json::json!(2_000_000.0);
    result.metrics[0].value.as_mut().unwrap().value = "2000000".to_owned();
    assert_eq!(
        statistic(&summary, &result, "factor-test", &settings)
            .unwrap()
            .value,
        Some(2_000_000.0)
    );
}
