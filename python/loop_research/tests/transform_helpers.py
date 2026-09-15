"""Invented source-to-worker fixtures for versioned factor transformations."""

import json
from pathlib import Path
from typing import Any

from loop.v1.common_pb2 import CivilDate, JobId, LeaseId, Sha256Digest
from loop.v1.evaluation_pb2 import FactorEvaluationWork
from loop.v1.factor_pb2 import FACTOR_DIRECTION_HIGHER_IS_BETTER, FactorSpec
from loop_protocol.canonical import FieldNode, canonicalize_expression
from loop_protocol.job import factor_spec_identity_sha256
from loop_protocol.provenance import PROVENANCE_COMPONENTS
from panel_helpers import Case, change

from loop_research.build_identity import BuildIdentity
from loop_research.data.fetch_cache import publish, read_cached
from loop_research.factor_worker import _artifact
from loop_research.operators import operator_registry
from loop_research.panel_builder import load_panel_request
from loop_research.panel_io import PanelManifest
from loop_research.panel_models import PanelReport
from loop_research.transform_models import TransformRequest

FIXTURES = Path(__file__).resolve().parents[3] / "fixtures/market/transforms"


def make_case(directory: Path) -> Case:
    sources, output = directory / "sources", directory / "derived"
    sources.mkdir(mode=0o700)
    output.mkdir(mode=0o700)
    for name in ("source.json", "capture.json", "exposures.json", "transform.json"):
        publish(sources, (FIXTURES / name).read_bytes())
    return Case(sources, output, load_panel_request(FIXTURES / "request.json"))


def recipe(case: Case) -> dict[str, Any]:
    assert case.request.transform is not None
    return json.loads(read_cached(case.sources, case.request.transform))  # type: ignore[no-any-return]


def replace_recipe(case: Case, value: dict[str, Any]) -> None:
    reference = publish(case.sources, json.dumps(value).encode())
    case.request = change(case.request, transform=reference.model_dump())


def risk_capture(case: Case) -> dict[str, Any]:
    parsed = TransformRequest.model_validate(recipe(case))
    assert parsed.exposure_capture is not None
    return json.loads(read_cached(case.sources, parsed.exposure_capture))  # type: ignore[no-any-return]


def replace_risk(case: Case, value: dict[str, Any]) -> None:
    reference = publish(case.sources, json.dumps(value).encode())
    current = recipe(case)
    current["exposure_capture"] = reference.model_dump()
    replace_recipe(case, current)


def work(
    case: Case, report: PanelReport, build: BuildIdentity, view: Path, output: Path
) -> FactorEvaluationWork:
    view.mkdir(mode=0o700)
    output.mkdir(mode=0o700)
    dataset = json.loads(read_cached(case.output, report.dataset))
    for artifact in dataset["snapshots"][0]["artifacts"]:
        source = case.output / artifact["object"]["sha256"][7:]
        destination = view / source.name
        destination.write_bytes(source.read_bytes())
        destination.chmod(0o444)
    view.chmod(0o555)
    manifest = PanelManifest.model_validate_json(read_cached(case.output, report.panel))
    assert manifest.transform is not None
    factor = FactorSpec(direction=FACTOR_DIRECTION_HIGHER_IS_BETTER)
    factor.expression.canonical_json = b'{"node":"field","field":"market.close"}'
    factor.expression.canonicalization_profile = "loop.factor-ast/v1"
    factor.expression.ast.schema_version = 1
    factor.expression.ast.root.field.field = "market.close"
    factor.expression_id.value = canonicalize_expression(
        FieldNode("market.close"), operator_registry()
    ).expression_id
    factor.expression.expression_id.CopyFrom(factor.expression_id)
    factor.operator_registry_sha256.value = bytes.fromhex(operator_registry().sha256[7:])
    for field in factor.frozen_policy.DESCRIPTOR.fields:
        policy = getattr(factor.frozen_policy, field.name)
        policy.policy_id.value = "policy." + field.name.removesuffix("_policy")
        policy.revision = "1"
        policy.sha256.value = b"p" * 32
    factor.frozen_policy.preprocess_policy.sha256.value = bytes.fromhex(
        manifest.transform.preprocess.digest()[7:]
    )
    factor.frozen_policy.neutralization_policy.sha256.value = bytes.fromhex(
        manifest.transform.neutralization.digest()[7:]
    )
    factor.factor_spec_id.value = "sha256:" + factor_spec_identity_sha256(factor).hex()
    request = FactorEvaluationWork(
        job_id=JobId(value="job.transform.01"),
        lease_id=LeaseId(value="lease.transform.01"),
        factor=factor,
        sample_start=CivilDate(year=2010, month=1, day=2),
        sample_end=CivilDate(year=2010, month=1, day=6),
        deterministic_seed=Sha256Digest(value=b"s" * 32),
    )
    for component in PROVENANCE_COMPONENTS:
        getattr(request.provenance, component + "_sha256").value = b"c" * 32
    request.provenance.source_code_sha256.value = bytes.fromhex(build.source["sha256"][7:])
    request.provenance.environment_sha256.value = bytes.fromhex(build.environment["sha256"][7:])
    request.provenance.operator_registry_sha256.CopyFrom(factor.operator_registry_sha256)
    request.panel_manifest.CopyFrom(
        _artifact(
            output,
            read_cached(case.output, report.panel),
            name="loop.factor_panel",
            media_type="application/json",
            columns=[],
            completed_ms=1_300_000_000_000,
            schema_version=2,
        )
    )
    return request
