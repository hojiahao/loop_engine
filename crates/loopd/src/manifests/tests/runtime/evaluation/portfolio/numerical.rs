//! The run recipe derives the same real inputs without restart-time job pins.

use std::collections::BTreeMap;

use serde::Serialize;

use super::*;
use crate::manifests::LocalArtifacts;
use crate::manifests::numerical::{NumericalResolver, PortfolioDerivation};
use crate::manifests::portfolio::PortfolioResolver;
use crate::store::BacktestPolicy;

#[derive(Serialize)]
struct Recipe<'a> {
    schema: &'static str,
    id: &'static str,
    revision: &'static str,
    evaluation_context: ObjectRef,
    direction: &'static str,
    deterministic_seed: String,
    execution_tape: &'a ObjectRef,
    policies: BTreeMap<String, String>,
    hypothesis: &'static str,
    required_observations: Vec<&'static str>,
    falsification_criteria: Vec<&'static str>,
}

#[tokio::test]
async fn derived_portfolio_replay() {
    let case = PortfolioCase::new().await;
    let fixture = &case.base.fixture;
    let source = &fixture.job.specification;
    let record = case
        .base
        .store
        .get(&source.job_id.as_ref().unwrap().value)
        .await
        .unwrap()
        .unwrap();
    let result = case
        .base
        .store
        .factor_trials(&actor(), &source.run_id.as_ref().unwrap().value, "", 10)
        .await
        .unwrap()
        .remove(0)
        .evaluation
        .unwrap()
        .result
        .unwrap();
    let job_specification::Input::FactorEvaluation(input) = source.input.as_ref().unwrap() else {
        panic!("evaluation source");
    };
    let dataset: model::Dataset =
        serde_json::from_slice(&std::fs::read(fixture.path(&fixture.context.data)).unwrap())
            .unwrap();
    let tape = dataset
        .snapshots
        .iter()
        .flat_map(|snapshot| &snapshot.artifacts)
        .find(|artifact| artifact.schema.name == "loop.execution_tape")
        .unwrap();
    let roles = [
        "universe",
        "data",
        "calendar",
        "preprocess",
        "neutralization",
        "portfolio",
        "execution",
        "cost",
        "evaluation",
    ];
    let recipe = fixture.json(&Recipe {
        schema: "loop.run-numerical/v1",
        id: "recipe.numerical",
        revision: "1",
        evaluation_context: fixture.json(&fixture.context),
        direction: "higher_is_better",
        deterministic_seed: format!("sha256:{}", "01".repeat(32)),
        execution_tape: &tape.object,
        policies: roles
            .into_iter()
            .map(|role| (format!("{role}_policy"), format!("policy.{role}")))
            .collect(),
        hypothesis: "A moving average ranks the synthetic cross section.",
        required_observations: vec!["Causal closing-price observations."],
        falsification_criteria: vec!["Coverage below the frozen threshold."],
    });
    let policy = PolicyReference {
        policy_id: Some(PolicyId {
            value: "recipe.numerical".to_owned(),
        }),
        revision: "1".to_owned(),
        sha256: Some(model::digest(recipe.digest().unwrap())),
    };
    let resolver = NumericalResolver::open(&fixture.root, &fixture.root).unwrap();
    let frozen = resolver
        .prepare(&policy, &recipe, input.dataset.as_ref().unwrap())
        .await
        .unwrap();
    let outputs = LocalArtifacts::open(&case.base.output).unwrap();
    let request = |replay| PortfolioDerivation {
        record: &record,
        result: &result,
        budget: input.budget.clone().unwrap(),
        job_id: "job.derived",
        backtest_id: "backtest.derived",
        outputs: &outputs,
        replay,
    };
    let derived = resolver.portfolio(&frozen, request(false)).await.unwrap();
    derived.check().unwrap();
    let replay = resolver.portfolio(&frozen, request(true)).await.unwrap();
    assert_eq!(derived.pin.specification, replay.pin.specification);
    assert_eq!(derived.pin.request, replay.pin.request);
    assert_eq!(derived.input, replay.input);
    let mut job = case.job.clone();
    job.job_id.as_mut().unwrap().value = "job.derived".to_owned();
    job.input = Some(job_specification::Input::Backtest(derived.input.clone()));
    let portfolios = PortfolioResolver::open(&fixture.root, vec![]).unwrap();
    assert!(portfolios.prepare(&job).await.is_err());
    let prepared = portfolios.prepare_pin(&derived.pin).await.unwrap();
    prepared.validate_inputs(&job).unwrap();
    prepared
        .bind_job(&job)
        .unwrap()
        .pending()
        .validate_inputs(&job)
        .unwrap();
    prepared
        .bind_job(&job)
        .unwrap()
        .pending()
        .validate_inputs(&job)
        .unwrap();
    job.job_id.as_mut().unwrap().value = "job.unrelated".to_owned();
    assert!(prepared.bind_job(&job).is_err());

    // A changed registered DTO cannot select an unverified result manifest.
    let mut changed = result.clone();
    changed.valid_observations -= 1;
    let rejected = PortfolioDerivation {
        result: &changed,
        ..request(true)
    };
    assert!(resolver.portfolio(&frozen, rejected).await.is_err());

    // Successful replays still require the exact source output bytes. They
    // neither trust the copied object nor recreate a missing published object.
    let values = result.values.as_ref().unwrap();
    let source_path = case
        .base
        .output
        .join(&values.artifact_id.as_ref().unwrap().value[7..]);
    let source_bytes = std::fs::read(&source_path).unwrap();
    std::fs::write(&source_path, b"corrupt").unwrap();
    assert!(resolver.portfolio(&frozen, request(true)).await.is_err());
    assert!(derived.check().is_err());
    std::fs::write(source_path, source_bytes).unwrap();
    std::fs::remove_file(fixture.path(&derived.pin.request)).unwrap();
    assert!(resolver.portfolio(&frozen, request(true)).await.is_err());
    assert!(!fixture.path(&derived.pin.request).exists());
}
