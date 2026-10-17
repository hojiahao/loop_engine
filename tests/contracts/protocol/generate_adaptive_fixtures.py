"""Reproduce the bounded v3 wire golden without executing any research work."""

import json
from pathlib import Path

from loop.runs.v1 import service_pb2 as runs
from loop.v1 import artifact_pb2, common_pb2, development_data_pb2, job_pb2, perturbation_pb2
from loop_protocol.job import protocol_selection_sha256, validate_job_specification
from loop_protocol.runs import validate_view


def digest(value: int) -> common_pb2.Sha256Digest:
    return common_pb2.Sha256Digest(value=bytes([value]) * 32)


def main() -> None:
    root = Path(__file__).resolve().parents[3]
    directory = root / "fixtures/contracts/protocol/v1"
    numerical = json.loads((directory / "run_view_v2.json").read_text())
    view = runs.RunView.FromString(bytes.fromhex(numerical["wire_hex"]))
    view.maximum_rounds = 3
    view.research.stage = runs.RUN_STAGE_PERTURBATION
    view.research.ClearField("discovery_job_id")
    view.research.ClearField("evaluation_job_id")
    view.research.ClearField("expression_id")
    view.research.ClearField("factor_spec_id")
    view.research.perturbation_job_id.value = "job.perturbation.2"
    view.research.research.job_id.CopyFrom(view.research.perturbation_job_id)
    policy = common_pb2.PolicyReference(
        policy_id=common_pb2.PolicyId(value="feedback.fixture"), revision="1", sha256=digest(7)
    )
    view.adaptive.CopyFrom(
        runs.AdaptiveRunView(
            feedback_policy=policy,
            source_count=1,
            feedback_sha256=digest(8),
            model_context_sha256=digest(9),
            family_sha256=digest(10),
            state_sha256=digest(11),
            random_draws=1,
        )
    )
    validate_view(view)
    selection = common_pb2.ProtocolSelectionSnapshot(
        selected_package="loop.v1",
        enabled_features=[
            "jobs.envelope.v1",
            "jobs.kind-input.v1",
            "jobs.perturbation.v1",
            "jobs.prelease-terminal.v1",
        ],
        effective_limits=common_pb2.ProtocolLimits(
            maximum_unary_bytes=4194304,
            maximum_stream_event_bytes=1048576,
            maximum_canonical_ast_bytes=262144,
            maximum_ast_nodes=4096,
            maximum_ast_depth=64,
            maximum_page_records=500,
            maximum_identity_bytes=128,
            maximum_artifact_uri_bytes=2048,
        ),
        server_build_version="0.2.0",
        client_build_version="0.2.0",
        server_build_sha256=digest(12),
        client_build_sha256=digest(13),
        schema_descriptor_sha256=digest(14),
    )
    selection.selected_at.seconds = 1
    selection.selection_sha256.value = protocol_selection_sha256(selection)
    work = artifact_pb2.ArtifactRef(
        artifact_id=common_pb2.ArtifactId(value="sha256:" + digest(15).value.hex()),
        uri="artifact://sha256/" + digest(15).value.hex(),
        sha256=digest(15),
        schema=artifact_pb2.ArtifactSchemaReference(
            name="loop.perturbation_work",
            version=1,
            schema_sha256=digest(16),
        ),
        media_type="application/x-protobuf",
        byte_size=100,
    )
    work.created_at.seconds = 1
    budget = job_pb2.JobBudget(
        maximum_steps=1,
        maximum_cost=common_pb2.Money(
            currency_code="USD",
            amount=common_pb2.ExactDecimal(value="0"),
        ),
    )
    budget.maximum_wall_time.seconds = 20
    job = job_pb2.JobSpecification(
        job_id=common_pb2.JobId(value="job.perturbation.2"),
        run_id=view.run_id,
        kind=job_pb2.JOB_KIND_PERTURBATION,
        perturbation=job_pb2.PerturbationJobInput(
            dataset=development_data_pb2.DevelopmentDatasetReference(
                snapshot_ids=[common_pb2.SnapshotId(value="data.development")],
                manifest_sha256=digest(17),
            ),
            feedback_policy=policy,
            parent_binding_sha256=digest(18),
            family_sha256=digest(10),
            work=work,
            source_code_sha256=digest(19),
            environment_sha256=digest(20),
            budget=budget,
            algorithm="window.ema-gradient-pcg64.v1",
        ),
        submitted_by=common_pb2.Actor(
            actor_id=common_pb2.ActorId(value="research.fixture"),
            kind=common_pb2.ACTOR_KIND_SERVICE,
            authenticated_subject="service:research.fixture",
        ),
        idempotency_key=common_pb2.IdempotencyKey(value="perturbation.fixture"),
        correlation_id=common_pb2.CorrelationId(value="correlation.fixture"),
        causation_id=common_pb2.CausationId(value="causation.fixture"),
        protocol_selection=selection,
    )
    job.submitted_at.seconds = 2
    validate_job_specification(job)
    identities = [
        common_pb2.FactorSpecId(value="sha256:" + digest(value).value.hex()) for value in (21, 22)
    ]
    observed = perturbation_pb2.PerturbationWork(
        state=perturbation_pb2.PerturbationState(version=1, random_seed=digest(23)),
        candidates=[
            perturbation_pb2.WindowCandidate(window=window, factor_spec_id=identity)
            for window, identity in zip((2, 5), identities, strict=True)
        ],
        current_window=5,
        observed_factor_ids=identities,
    )
    fixture = {
        "schema": "loop.adaptive-contract/v1",
        "view_wire_hex": view.SerializeToString(deterministic=True).hex(),
        "job_wire_hex": job.SerializeToString(deterministic=True).hex(),
        "observed_work_wire_hex": observed.SerializeToString(deterministic=True).hex(),
    }
    (directory / "adaptive_workflow_v3.json").write_text(json.dumps(fixture, indent=2) + "\n")


if __name__ == "__main__":
    main()
