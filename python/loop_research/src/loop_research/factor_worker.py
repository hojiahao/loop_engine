"""Fixed numerical subprocess; orchestration owns authentication and completion."""

from __future__ import annotations

import argparse
import csv
import io
import math
import re
import sys
import time
from dataclasses import dataclass
from datetime import date
from pathlib import Path

from google.protobuf.message import DecodeError  # type: ignore[import-untyped]
from google.protobuf.timestamp_pb2 import Timestamp  # type: ignore[import-untyped]
from loop.v1.artifact_pb2 import ArtifactRef, ArtifactSchemaReference
from loop.v1.common_pb2 import ArtifactId, Sha256Digest
from loop.v1.evaluation_pb2 import FactorEvaluationResult, FactorEvaluationWork
from loop_protocol.artifact import validate_artifact_ref
from loop_protocol.canonical import CanonicalFactorSpec, parse_factor_spec
from loop_protocol.job import (
    factor_identity_bytes,
    validate_factor_identity,
)
from loop_protocol.provenance import PROVENANCE_COMPONENTS, ProvenanceSnapshot

from loop_research.build_identity import canonical_bytes, publish_object, require_build
from loop_research.cross_section import transform
from loop_research.evaluator import Evaluation, evaluate
from loop_research.operators import operator_registry
from loop_research.panel_io import ContentRef, PanelInput, load_panel
from loop_research.transform_models import TransformEvidence, resolve_policy

MAX_MESSAGE_BYTES = 1_048_576
MAX_OUTPUT_BYTES = 64 * 1024 * 1024
_IDENTIFIER = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}\Z", re.ASCII)


def _require_build(work: FactorEvaluationWork) -> None:
    require_build(
        "sha256:" + work.provenance.source_code_sha256.value.hex(),
        "sha256:" + work.provenance.environment_sha256.value.hex(),
        profile="evaluation",
    )


def encode_values(result: Evaluation, loaded: PanelInput) -> bytes:
    """Encode the exact evaluated grid for publication or read-only replay."""
    stream = io.StringIO(newline="")
    writer = csv.writer(stream, lineterminator="\n")
    writer.writerow(["session", "security_id", "eligible", "value"])
    start = loaded.panel.sessions.index(result.evaluation_start)
    for index, row in enumerate(result.values):
        for column, value in enumerate(row):
            writer.writerow(
                [
                    loaded.panel.sessions[start + index].isoformat(),
                    loaded.panel.securities[column],
                    "1" if loaded.panel.eligible[start + index, column] else "0",
                    format(float(value), ".17g") if math.isfinite(float(value)) else "",
                ]
            )
        if stream.tell() > MAX_OUTPUT_BYTES:
            raise ValueError("factor output byte budget exceeded")
    return stream.getvalue().encode("ascii")


def _artifact(
    output: Path,
    content: bytes,
    *,
    name: str,
    media_type: str,
    columns: list[str],
    completed_ms: int,
    row_count: int | None = None,
    schema_version: int = 1,
) -> ArtifactRef:
    schema = publish_object(
        output,
        canonical_bytes(
            {
                "schema": "loop.artifact-schema/v1",
                "name": name,
                "version": schema_version,
                "media_type": media_type,
                "columns": columns,
            }
        ),
    )
    reference = publish_object(output, content)
    digest = bytes.fromhex(reference["sha256"][7:])
    result = ArtifactRef(
        artifact_id=ArtifactId(value=reference["sha256"]),
        uri="artifact://sha256/" + digest.hex(),
        sha256=Sha256Digest(value=digest),
        schema=ArtifactSchemaReference(
            name=name,
            version=schema_version,
            schema_sha256=Sha256Digest(value=bytes.fromhex(schema["sha256"][7:])),
        ),
        media_type=media_type,
        byte_size=reference["byte_size"],
        created_at=Timestamp(seconds=completed_ms // 1000, nanos=(completed_ms % 1000) * 1_000_000),
    )
    if row_count is not None:
        result.row_count = row_count
    validate_artifact_ref(result)
    return result


@dataclass(frozen=True, slots=True)
class FactorComputation:
    """Verified numerical inputs and values, without publication or job authority."""

    factor: CanonicalFactorSpec
    loaded: PanelInput
    result: Evaluation
    transformation: TransformEvidence | None
    values_csv: bytes


def compute(work: FactorEvaluationWork, *, view: Path) -> FactorComputation:
    """Recompute the frozen factor from actual files, without writing artifacts.

    An administrative replay may use this same kernel. It does not authenticate
    the caller or turn a supplied job/lease ID into execution authority.
    """
    if work.ByteSize() > MAX_MESSAGE_BYTES:
        raise ValueError("factor work message budget")
    if not _IDENTIFIER.fullmatch(work.job_id.value) or not _IDENTIFIER.fullmatch(
        work.lease_id.value
    ):
        raise ValueError("factor work requires job and lease identities")
    ProvenanceSnapshot.from_wire(work.provenance)
    if len(work.deterministic_seed.value) != 32:
        raise ValueError("factor work requires a deterministic seed")
    validate_factor_identity(work.factor)
    factor = parse_factor_spec(
        factor_identity_bytes(work.factor),
        work.factor.factor_spec_id.value,
        work.factor.expression.canonical_json,
        operator_registry(),
    )
    if work.provenance.operator_registry_sha256 != work.factor.operator_registry_sha256:
        raise ValueError("factor work operator provenance mismatch")
    panel_reference = validate_artifact_ref(work.panel_manifest)
    if (
        panel_reference.schema_name != "loop.factor_panel"
        or panel_reference.schema_version not in {1, 2}
        or panel_reference.media_type != "application/json"
    ):
        raise ValueError("factor work requires a supported panel manifest")
    _require_build(work)
    loaded = load_panel(
        view,
        ContentRef(sha256=panel_reference.artifact_id, byte_size=panel_reference.byte_size),
        sample_start=date(work.sample_start.year, work.sample_start.month, work.sample_start.day),
        sample_end=date(work.sample_end.year, work.sample_end.month, work.sample_end.day),
    )
    processing = loaded.manifest.transform
    output_version = 1 if processing is None else 2
    if panel_reference.schema_version != output_version:
        raise ValueError("panel artifact and transformation version differ")
    if processing is not None:
        for reference, policy_document in (
            (factor.spec.preprocess_policy, processing.preprocess),
            (factor.spec.neutralization_policy, processing.neutralization),
        ):
            if (reference.policy_id, reference.revision, reference.sha256) != (
                policy_document.policy_id,
                policy_document.revision,
                policy_document.digest(),
            ):
                raise ValueError("transformation policy differs from the frozen FactorSpec")
    result = evaluate(
        factor, loaded.panel, evaluation_start=date.fromisoformat(loaded.manifest.evaluation_start)
    )
    transformation = None
    if processing is not None:
        transformed = transform(
            result,
            loaded.panel,
            resolve_policy(processing.preprocess, processing.neutralization),
            loaded.exposures,
        )
        result = transformed.evaluation
        transformation = TransformEvidence(
            preprocess_sha256=processing.preprocess.digest(),
            neutralization_sha256=processing.neutralization.digest(),
            exposures_sha256=processing.exposures.sha256
            if processing.exposures is not None
            else None,
            raw_valid_observations=transformed.raw_valid_observations,
            outcomes=transformed.outcomes,
        )
    content = encode_values(result, loaded)
    loaded.check()
    _require_build(work)
    return FactorComputation(factor, loaded, result, transformation, content)


def encode_manifest(
    work: FactorEvaluationWork,
    computed: FactorComputation,
    *,
    values_sha256: str,
    completed_ms: int,
) -> bytes:
    """Preserve the versioned result format in both publication and verification."""
    result, loaded = computed.result, computed.loaded
    transformation = computed.transformation
    output_version = 1 if transformation is None else 2
    return canonical_bytes(
        {
            "schema": f"loop.factor-evaluation-result/v{output_version}",
            "job_id": work.job_id.value,
            "lease_id": work.lease_id.value,
            "factor_spec_id": result.factor_spec_id,
            "expression_id": result.expression_id,
            "provenance": {
                component + "_sha256": "sha256:"
                + getattr(work.provenance, component + "_sha256").value.hex()
                for component in PROVENANCE_COMPONENTS
            },
            "deterministic_seed": "sha256:" + work.deterministic_seed.value.hex(),
            "panel_manifest_sha256": work.panel_manifest.artifact_id.value,
            "values_sha256": values_sha256,
            "quality": loaded.manifest.quality,
            "sample_start": date(
                work.sample_start.year, work.sample_start.month, work.sample_start.day
            ).isoformat(),
            "sample_end": date(
                work.sample_end.year, work.sample_end.month, work.sample_end.day
            ).isoformat(),
            "eligible_observations": result.eligible_observations,
            "valid_observations": result.valid_observations,
            "work_units": result.work_units,
            "completed_at_ms": completed_ms,
            **(
                {"transform": transformation.model_dump(mode="json")}
                if transformation is not None
                else {}
            ),
        }
    )


def execute(work: FactorEvaluationWork, *, view: Path, output: Path) -> FactorEvaluationResult:
    """Compute from a runtime-prepared leaf and publish immutable candidate output.

    The trusted launcher owns the paths and authenticated lease. This function
    has no provider, database, holdout or network API. Its response is numerical
    evidence, not permission to complete a job or release results. The runtime
    must recheck frozen manifests, the current lease and receipt transaction.
    """
    if not output.is_absolute() or output.resolve(strict=True) != output:
        raise ValueError("factor output must be a canonical deployment directory")
    if output == view or output.is_relative_to(view) or view.is_relative_to(output):
        raise ValueError("factor output must be separate from its data view")
    computed = compute(work, view=view)
    result, loaded = computed.result, computed.loaded
    output_version = 1 if computed.transformation is None else 2
    content = computed.values_csv
    completed_ms = time.time_ns() // 1_000_000
    values = _artifact(
        output,
        content,
        name="loop.factor_values",
        media_type="text/csv",
        columns=["session", "security_id", "eligible", "value"],
        completed_ms=completed_ms,
        row_count=result.values.size,
        schema_version=output_version,
    )
    report = FactorEvaluationResult(
        job_id=work.job_id,
        lease_id=work.lease_id,
        factor_spec_id=work.factor.factor_spec_id,
        expression_id=work.factor.expression_id,
        provenance=work.provenance,
        deterministic_seed=work.deterministic_seed,
        values=values,
        eligible_observations=result.eligible_observations,
        valid_observations=result.valid_observations,
        work_units=result.work_units,
        sample_start=work.sample_start,
        sample_end=work.sample_end,
        completed_at=values.created_at,
    )
    document = encode_manifest(
        work, computed, values_sha256=values.artifact_id.value, completed_ms=completed_ms
    )
    report.manifest.CopyFrom(
        _artifact(
            output,
            document,
            name="loop.factor_evaluation",
            media_type="application/json",
            columns=[
                "factor_spec_id",
                "values_sha256",
                "valid_observations",
                "eligible_observations",
            ],
            completed_ms=completed_ms,
            schema_version=output_version,
        )
    )
    loaded.check()
    return report


def main() -> int:
    """One bounded Protobuf exchange; failures emit no successful result."""
    parser = argparse.ArgumentParser(prog="loop-research-factor-worker")
    parser.add_argument("--view", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify-build", action="store_true")
    arguments = parser.parse_args()
    try:
        payload = sys.stdin.buffer.read(MAX_MESSAGE_BYTES + 1)
        if not payload or len(payload) > MAX_MESSAGE_BYTES:
            raise ValueError("factor work message budget")
        work = FactorEvaluationWork.FromString(payload)
        if arguments.verify_build:
            ProvenanceSnapshot.from_wire(work.provenance)
            _require_build(work)
            return 0
        result = execute(work, view=arguments.view, output=arguments.output)
        encoded = result.SerializeToString()
        if len(encoded) > MAX_MESSAGE_BYTES:
            raise ValueError("factor result message budget")
    except ValueError, DecodeError, OSError, OverflowError, csv.Error:
        sys.stderr.write("factor evaluation failed; no completion authorized\n")
        return 2
    sys.stdout.buffer.write(encoded)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
