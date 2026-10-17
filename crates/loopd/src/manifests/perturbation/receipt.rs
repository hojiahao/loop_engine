use super::*;

/// Output from one supervised installed-worker invocation. The runtime supplies
/// its actual lease record and checks authority again before committing it.
pub(crate) struct PerturbationCompletion {
    pub record: wire::JobRecord,
    pub step: wire::PerturbationStep,
    pub build: ResearchBuild,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    leased_record: ObjectRef,
    work: ObjectRef,
    step: ObjectRef,
    lease_id: String,
    started_at_ms: i64,
    completed_at_ms: i64,
    algorithm: String,
    family_sha256: String,
    source_code_sha256: String,
    environment_sha256: String,
    source_jobs: Vec<String>,
}

/// Unforgeable within normal caller APIs: only validated prepared inputs and
/// actual immutable output bytes construct this completion proof.
pub(crate) struct PerturbationEvidence {
    prepared: Arc<PreparedPerturbation>,
    job: wire::JobSpecification,
    lease_id: String,
    leased: wire::JobRecord,
    step: wire::PerturbationStep,
    started_at_ms: i64,
    completed_at_ms: i64,
    report: wire::ArtifactRef,
    step_artifact: wire::ArtifactRef,
    files: Vec<Arc<VerifiedFile>>,
}

impl PreparedPerturbation {
    /// Seal one installed result. This publishes evidence, not a successful job;
    /// the transaction must still fence the exact lease and source records.
    pub(crate) async fn seal(
        self: Arc<Self>,
        resolver: &NumericalResolver,
        completion: PerturbationCompletion,
    ) -> StoreResult<PerturbationEvidence> {
        self.check()?;
        validate_output(&self, &completion)?;
        let mut files = Vec::new();
        let lease_id = lease(&completion.record)?;
        let leased_record = resolver
            .publish(&completion.record.encode_to_vec(), false, &mut files)
            .await?;
        let step = resolver
            .publish(&completion.step.encode_to_vec(), false, &mut files)
            .await?;
        let receipt = Receipt {
            schema: "loop.perturbation-receipt/v1".to_owned(),
            leased_record,
            work: self.reference.clone(),
            step: step.clone(),
            lease_id: lease_id.to_owned(),
            started_at_ms: completion.started_at_ms,
            completed_at_ms: completion.completed_at_ms,
            algorithm: self.input.algorithm.clone(),
            family_sha256: digest(self.family.digest()),
            source_code_sha256: digest(self.build.source_sha256),
            environment_sha256: digest(self.build.environment_sha256),
            source_jobs: self
                .observations
                .iter()
                .map(|source| source.entry().job_id.clone())
                .collect(),
        };
        let object = resolver.json(&receipt, false, &mut files).await?;
        let report = artifact(
            resolver,
            object,
            "loop.perturbation_receipt",
            "application/json",
            completion.completed_at_ms,
            false,
            &mut files,
        )
        .await?;
        let step_artifact = artifact(
            resolver,
            step,
            "loop.perturbation_step",
            "application/x-protobuf",
            completion.completed_at_ms,
            false,
            &mut files,
        )
        .await?;
        let evidence = PerturbationEvidence {
            job: completion
                .record
                .specification
                .clone()
                .ok_or(StoreError::Corrupt("perturbation lease job"))?,
            lease_id: receipt.lease_id,
            prepared: self,
            leased: completion.record,
            step: completion.step,
            started_at_ms: completion.started_at_ms,
            completed_at_ms: completion.completed_at_ms,
            report,
            step_artifact,
            files,
        };
        evidence.check()?;
        Ok(evidence)
    }

    /// Reconstruct a committed completion without invoking the worker or
    /// republishing any missing byte. Every source proof remains mandatory.
    pub(crate) async fn recover(
        self: Arc<Self>,
        resolver: &NumericalResolver,
        record: &wire::JobRecord,
    ) -> StoreResult<PerturbationEvidence> {
        self.check()?;
        let Some(wire::job_outcome::Outcome::Success(success)) = record
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.outcome.as_ref())
        else {
            return Err(StoreError::AdmissionDenied);
        };
        let [report, step_artifact] = success.outputs.as_slice() else {
            return Err(StoreError::Corrupt("perturbation output count"));
        };
        let mut files = Vec::new();
        let reference = object(report, "loop.perturbation_receipt", "application/json")?;
        let file = resolver
            .artifacts
            .load(&reference, true, &mut ReadBudget::new())
            .await?;
        let receipt: Receipt = file.json()?;
        files.push(file);
        model::schema(&receipt.schema, "loop.perturbation-receipt/v1")?;
        if receipt.work != self.reference
            || receipt.algorithm != self.input.algorithm
            || receipt.family_sha256 != digest(self.family.digest())
            || receipt.source_code_sha256 != digest(self.build.source_sha256)
            || receipt.environment_sha256 != digest(self.build.environment_sha256)
            || receipt.source_jobs
                != self
                    .observations
                    .iter()
                    .map(|source| source.entry().job_id.clone())
                    .collect::<Vec<_>>()
            || receipt.step
                != object(
                    step_artifact,
                    "loop.perturbation_step",
                    "application/x-protobuf",
                )?
        {
            return Err(StoreError::Corrupt("perturbation receipt binding"));
        }
        let leased_file = resolver
            .artifacts
            .load(&receipt.leased_record, true, &mut ReadBudget::new())
            .await?;
        let leased = decode::<wire::JobRecord>(leased_file.bytes()?)?;
        files.push(leased_file);
        let step_file = resolver
            .artifacts
            .load(&receipt.step, true, &mut ReadBudget::new())
            .await?;
        let step = decode::<wire::PerturbationStep>(step_file.bytes()?)?;
        files.push(step_file);
        if lease(&leased)? != receipt.lease_id {
            return Err(StoreError::Corrupt("perturbation receipt lease"));
        }
        let completion = PerturbationCompletion {
            record: leased,
            step,
            build: self.build,
            started_at_ms: receipt.started_at_ms,
            completed_at_ms: receipt.completed_at_ms,
        };
        validate_output(&self, &completion)?;
        if artifact(
            resolver,
            reference,
            "loop.perturbation_receipt",
            "application/json",
            receipt.completed_at_ms,
            true,
            &mut files,
        )
        .await?
            != *report
            || artifact(
                resolver,
                receipt.step,
                "loop.perturbation_step",
                "application/x-protobuf",
                receipt.completed_at_ms,
                true,
                &mut files,
            )
            .await?
                != *step_artifact
        {
            return Err(StoreError::Corrupt("perturbation output envelope"));
        }
        let evidence = PerturbationEvidence {
            job: completion
                .record
                .specification
                .clone()
                .ok_or(StoreError::Corrupt("perturbation lease job"))?,
            lease_id: receipt.lease_id,
            prepared: self,
            leased: completion.record,
            step: completion.step,
            started_at_ms: completion.started_at_ms,
            completed_at_ms: completion.completed_at_ms,
            report: report.clone(),
            step_artifact: step_artifact.clone(),
            files,
        };
        evidence.check_record(record)?;
        Ok(evidence)
    }
}

impl PerturbationEvidence {
    pub(crate) fn check(&self) -> StoreResult<()> {
        self.prepared.check()?;
        check_files(&self.files)
    }
    pub(crate) fn job(&self) -> &wire::JobSpecification {
        &self.job
    }
    pub(crate) fn lease_id(&self) -> &str {
        &self.lease_id
    }
    pub(crate) fn started_at(&self) -> i64 {
        self.started_at_ms
    }
    pub(crate) fn sources(&self) -> &[(wire::JobRecord, wire::BacktestResult)] {
        self.prepared.sources()
    }
    pub(crate) fn step(&self) -> &wire::PerturbationStep {
        &self.step
    }
    pub(crate) fn success(&self) -> StoreResult<wire::JobSuccess> {
        self.check()?;
        Ok(wire::JobSuccess {
            outputs: vec![self.report.clone(), self.step_artifact.clone()],
        })
    }
    pub(crate) fn check_lease(&self, record: &wire::JobRecord) -> StoreResult<()> {
        self.check()?;
        if record != &self.leased {
            return Err(StoreError::Corrupt("perturbation active lease"));
        }
        Ok(())
    }
    pub(crate) fn check_record(&self, record: &wire::JobRecord) -> StoreResult<()> {
        self.check()?;
        if record.specification.as_ref() != Some(self.job())
            || record.state != wire::JobState::Succeeded as i32
            || Some(record.revision) != self.leased.revision.checked_add(1)
            || record.attempt != self.leased.attempt
            || record.active_lease.is_some()
            || record
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.outcome.as_ref())
                != Some(&wire::job_outcome::Outcome::Success(self.success()?))
            || record
                .updated_at
                .as_ref()
                .map(|value| crate::store::timestamp_millis(value, false))
                .transpose()?
                .is_none_or(|value| value < self.completed_at_ms)
        {
            return Err(StoreError::Corrupt("perturbation completed record"));
        }
        Ok(())
    }
}

fn validate_output(
    prepared: &PreparedPerturbation,
    completion: &PerturbationCompletion,
) -> StoreResult<()> {
    let record = &completion.record;
    prepared.bind_job(
        record
            .specification
            .as_ref()
            .ok_or(StoreError::Corrupt("perturbation lease job"))?,
    )?;
    let active = record
        .active_lease
        .as_ref()
        .ok_or(StoreError::Corrupt("perturbation lease"))?;
    let issued = active
        .issued_at
        .as_ref()
        .map(|value| crate::store::timestamp_millis(value, false))
        .transpose()?
        .ok_or(StoreError::Corrupt("perturbation lease time"))?;
    let expires = active
        .expires_at
        .as_ref()
        .map(|value| crate::store::timestamp_millis(value, false))
        .transpose()?
        .ok_or(StoreError::Corrupt("perturbation lease expiry"))?;
    if !matches!(
        wire::JobState::try_from(record.state),
        Ok(wire::JobState::Leased | wire::JobState::Running)
    ) || record.outcome.is_some()
        || record.revision == 0
        || record.attempt == 0
        || active.job_id
            != record
                .specification
                .as_ref()
                .and_then(|job| job.job_id.clone())
        || completion.build != prepared.build
        || completion.started_at_ms != issued
        || completion.completed_at_ms < completion.started_at_ms
        || completion.completed_at_ms >= expires
    {
        return Err(StoreError::Corrupt("perturbation execution binding"));
    }
    crate::store::validate_id(lease(record)?)?;
    crate::store::validate_perturbation(&prepared.work, prepared.family.space(), &completion.step)
}

fn lease(record: &wire::JobRecord) -> StoreResult<&str> {
    record
        .active_lease
        .as_ref()
        .and_then(|lease| lease.lease_id.as_ref())
        .map(|id| id.value.as_str())
        .ok_or(StoreError::Corrupt("perturbation lease ID"))
}

fn digest(value: [u8; 32]) -> String {
    format!(
        "sha256:{}",
        value
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

pub(super) fn object(
    reference: &wire::ArtifactRef,
    name: &str,
    media_type: &str,
) -> StoreResult<ObjectRef> {
    loop_protocol::artifact::validate_artifact_ref(reference)
        .map_err(|_| StoreError::Corrupt("perturbation artifact"))?;
    if reference
        .schema
        .as_ref()
        .is_none_or(|schema| schema.name != name || schema.version != 1)
        || reference.media_type != media_type
    {
        return Err(StoreError::Corrupt("perturbation artifact type"));
    }
    Ok(ObjectRef {
        sha256: reference
            .artifact_id
            .as_ref()
            .ok_or(StoreError::Corrupt("perturbation artifact ID"))?
            .value
            .clone(),
        byte_size: reference.byte_size,
    })
}

pub(super) fn decode<T: Message + Default>(bytes: &[u8]) -> StoreResult<T> {
    let value = T::decode(bytes).map_err(|_| StoreError::Corrupt("perturbation protobuf"))?;
    if value.encode_to_vec() != bytes {
        return Err(StoreError::Corrupt("noncanonical perturbation protobuf"));
    }
    Ok(value)
}
