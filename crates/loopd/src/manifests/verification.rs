use loop_core::factor::{
    CanonicalDecimal, FactorExpr, FactorSpec, FactorSpecId, Literal, OperatorCall, WindowPolicy,
    factor_spec_id,
};
use loop_protocol::wire::v1::{
    JobSpecification, PerturbationSpace, PolicyId, PolicyReference, WindowCandidate,
    job_specification,
};

use super::loading::{Materializer, ResolvedContext, policies, sorted};
use super::{model, model::parse_digest};
use crate::store::{AdmissionEvidence, StoreError, StoreResult};

pub(super) fn frozen_job(
    job: &JobSpecification,
    specification: &model::Backtest,
    context: &ResolvedContext,
    factor: &FactorSpec,
) -> StoreResult<()> {
    let Some(job_specification::Input::Backtest(input)) = &job.input else {
        return Err(StoreError::AdmissionDenied);
    };
    model::schema(&specification.schema, "loop.backtest-spec/v1")?;
    model::text(&specification.backtest_id)?;
    model::text(&specification.engine_version)?;
    specification.sample.validate()?;
    if specification.sample.role == model::SampleRole::OperatorWarmup
        || specification.engine != context.engine
        || specification.engine_version != context.engine_version
        || specification.sample != context.dataset.sample
        || specification.return_definition != "simple_nav_return"
        || input.return_definition
            != loop_protocol::wire::v1::ReturnDefinition::SimpleNavReturn as i32
        || input.factor_spec_id.as_ref().map(|id| id.value.as_str())
            != Some(factor_spec_id(factor).to_string().as_str())
        || input.dataset.as_ref() != Some(&context.dataset.reference(&context.manifest.data)?)
        || input.provenance.as_ref() != Some(&context.provenance)
        || input
            .deterministic_seed
            .as_ref()
            .map(|seed| seed.value.as_slice())
            != Some(parse_digest(&specification.deterministic_seed)?.as_slice())
    {
        return Err(StoreError::Corrupt("frozen backtest input binding"));
    }
    Ok(())
}

pub(super) async fn family(
    materializer: &mut Materializer<'_>,
    context: &ResolvedContext,
) -> StoreResult<Option<PerturbationSpace>> {
    let Some(reference) = &context.manifest.family else {
        return Ok(None);
    };
    if context.dataset.sample.role != model::SampleRole::InSample {
        return Err(StoreError::AdmissionDenied);
    }
    let family: model::Family = materializer.json(reference).await?;
    model::schema(&family.schema, "loop.window-family/v1")?;
    if family.algorithm != "window.ema-gradient-pcg64.v1"
        || family.window_path.is_empty()
        || family.window_path.len() > 32
        || family.candidates.len() < 2
        || family.candidates.len() > 64
    {
        return Err(StoreError::Corrupt("perturbation family bounds"));
    }
    let mut previous_window = 0;
    let mut baseline: Option<(FactorSpec, FactorExpr)> = None;
    let mut candidates = Vec::new();
    for candidate in family.candidates {
        if candidate.window <= previous_window || candidate.window > 4096 {
            return Err(StoreError::Corrupt("perturbation window order"));
        }
        previous_window = candidate.window;
        let factor = materializer.factor(&candidate.factor, context).await?;
        let normalized = mask_window(
            factor.canonical_expression(),
            &family.window_path,
            candidate.window,
            context,
        )?;
        if let Some((base, expression)) = &baseline {
            if normalized != *expression
                || policies(&factor) != policies(base)
                || factor.direction() != base.direction()
                || factor.operator_registry_sha256() != base.operator_registry_sha256()
            {
                return Err(StoreError::Corrupt("family changes more than one window"));
            }
        } else {
            baseline = Some((factor.clone(), normalized));
        }
        candidates.push(WindowCandidate {
            window: candidate.window,
            factor_spec_id: Some(loop_protocol::wire::v1::FactorSpecId {
                value: factor_spec_id(&factor).to_string(),
            }),
        });
    }
    Ok(Some(PerturbationSpace {
        algorithm: family.algorithm,
        dataset: Some(context.dataset.reference(&context.manifest.data)?),
        provenance: Some(context.provenance.clone()),
        backtest_seed: Some(model::digest(parse_digest(&family.backtest_seed)?)),
        random_seed: Some(model::digest(parse_digest(&family.random_seed)?)),
        candidates,
    }))
}

fn mask_window(
    expression: &FactorExpr,
    path: &[usize],
    window: u32,
    context: &ResolvedContext,
) -> StoreResult<FactorExpr> {
    let FactorExpr::Call(call) = expression else {
        return Err(StoreError::Corrupt("window parent"));
    };
    let Some((&index, rest)) = path.split_first() else {
        return Err(StoreError::Corrupt("window path"));
    };
    let mut arguments = call.arguments().to_vec();
    let child = arguments
        .get_mut(index)
        .ok_or(StoreError::Corrupt("window argument"))?;
    if rest.is_empty() {
        let semantic = context
            .registry
            .semantic_contract_for(call.operator())
            .ok_or(StoreError::Corrupt("window operator semantics"))?;
        if index != 1 || !matches!(semantic.window_policy(),
            WindowPolicy::TrailingArgument2FullWindowRightInclusiveConstantPreserve
                | WindowPolicy::TrailingArgument2MinimumValidMinNMax3Floor2NDiv3RightInclusiveConstantPreserve)
            || !matches!(child, FactorExpr::Literal(Literal::Decimal(value)) if value.as_str() == window.to_string())
        {
            return Err(StoreError::Corrupt("non-window perturbation argument"));
        }
        *child = FactorExpr::Literal(Literal::Decimal(
            CanonicalDecimal::new("1").map_err(|_| StoreError::Corrupt("window mask"))?,
        ));
    } else {
        *child = mask_window(child, rest, window, context)?;
    }
    Ok(FactorExpr::Call(OperatorCall::new(
        call.operator().clone(),
        arguments,
    )))
}

pub(super) async fn review(
    materializer: &mut Materializer<'_>,
    entry: &model::BacktestEntry,
    context: &ResolvedContext,
    factor: &FactorSpec,
) -> StoreResult<Option<AdmissionEvidence>> {
    let Some(reference) = &entry.review else {
        return Ok(None);
    };
    let result = entry
        .result
        .as_ref()
        .ok_or(StoreError::Corrupt("review without result"))?;
    if context.dataset.sample.role != model::SampleRole::InSample {
        return Err(StoreError::AdmissionDenied);
    }
    materializer.artifact(reference).await?;
    if reference.schema.name != "loop.admission_review"
        || reference.schema.version != 1
        || reference.media_type != "application/json"
    {
        return Err(StoreError::Corrupt("review artifact type"));
    }
    let report: model::Review = materializer.json(&reference.object).await?;
    model::schema(&report.schema, "loop.admission-review/v1")?;
    report.policy.validate()?;
    let policy = factor.evaluation_policy();
    if report.job_id != entry.job_id
        || report.result != result.object
        || report.factor_spec_id != factor_spec_id(factor).to_string()
        || report.policy.policy_id != policy.policy_id().as_str()
        || report.policy.revision != policy.revision().as_str()
        || report.policy.document.digest()? != *policy.sha256()
        || report.valid_observations > report.eligible_observations
        || report.eligible_observations == 0
        || !(1..=10_000).contains(&report.minimum_coverage_bps)
        || !matches!(
            report.machine_rejection.as_str(),
            "" | "deterministic_filter" | "performance" | "correlation" | "policy"
        )
        || reference.created_at_ms < result.created_at_ms
    {
        return Err(StoreError::Corrupt("review evidence binding"));
    }
    let configuration = context
        .policy_documents
        .iter()
        .find(|document| document.policy_id == report.policy.policy_id)
        .ok_or(StoreError::Corrupt("admission policy configuration"))?;
    if configuration.settings.get("minimum_coverage_bps")
        != Some(&report.minimum_coverage_bps.to_string())
    {
        return Err(StoreError::Corrupt("admission coverage policy"));
    }
    sorted(report.replacements.iter().map(String::as_str), 0, 16)?;
    for id in &report.replacements {
        FactorSpecId::parse(id).map_err(|_| StoreError::Corrupt("replacement factor ID"))?;
    }
    Ok(Some(AdmissionEvidence {
        report: Some(reference.wire()?),
        policy: Some(PolicyReference {
            policy_id: Some(PolicyId {
                value: report.policy.policy_id,
            }),
            revision: report.policy.revision,
            sha256: Some(model::digest(report.policy.document.digest()?)),
        }),
        result_manifest_sha256: result.object.digest()?.to_vec(),
        library_sha256: parse_digest(&report.library_sha256)?.to_vec(),
        eligible_observations: report.eligible_observations,
        valid_observations: report.valid_observations,
        minimum_coverage_bps: report.minimum_coverage_bps,
        machine_rejection: report.machine_rejection,
        semantic_accepted: report.semantic_accepted,
        replacements: report.replacements,
    }))
}
