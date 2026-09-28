import copy
import json
from pathlib import Path

import pytest
from loop.v1.factor_pb2 import FACTOR_DIRECTION_LOWER_IS_BETTER
from test_backtest_workflow import Case
from test_backtest_workflow import build as build
from test_backtest_workflow import case as case
from test_backtest_workflow import prepared as prepared
from test_global_statistics import parse, policy, work
from test_portfolio_worker import prepare as portfolio_work
from test_portfolio_worker import run
from test_statistics_workflow import extended as extended
from test_statistics_workflow import freeze

from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_records import CachedObject
from loop_research.global_models import GlobalWork
from loop_research.global_worker import historical_ledger, prepare
from loop_research.portfolio_worker import TrialLedger


def registered(extended: Case, *, incremental: bool = False) -> GlobalWork:
    """Reconstruct two real opposing strategies on one exact immutable panel."""
    second = copy.deepcopy(extended)
    second.work.factor.direction = FACTOR_DIRECTION_LOWER_IS_BETTER
    document = work().model_dump(mode="json", by_alias=True)
    recipes = []
    for index, current in enumerate((extended, second)):
        current.work.job_id.value = f"job.a.{index}"
        current.work.lease_id.value = f"lease.a.{index}"
        if index == 0:
            recipe = portfolio_work(current)
        else:
            # The second strategy shares the already broker-published read-only
            # market view. Only its factor evaluation and request are different.
            freeze(current)
            recipe = recipes[0].model_copy(
                update={
                    "request": publish(
                        current.evidence,
                        canonical_bytes(current.request.model_dump(mode="json", by_alias=True)),
                    )
                }
            )
        recipes.append(recipe)
        for entry in document["snapshot"]["ledger"]["entries"]:
            if entry["job_id"] in (f"job.a.{index}", f"job.b.{index}"):
                entry["factor_spec_id"] = current.work.factor.factor_spec_id.value
    value = parse(document)
    for index, (current, recipe) in enumerate(zip((extended, second), recipes, strict=True)):
        source = value.portfolios[index]
        ledger = value.snapshot.ledger
        if incremental and index == 0:
            ledger = TrialLedger(
                schema="loop.global-trials/v1",
                entries=tuple(
                    entry for entry in ledger.entries if entry.job_id in ("job.a.0", "job.b.0")
                ),
            )
        recipe = recipe.model_copy(
            update={
                "job_id": source.job_id,
                "lease_id": source.lease_id,
                "trials": ledger,
            }
        )
        artifact = run(current, recipe)
        document["portfolios"][index].update(
            request=recipe.request.model_dump(),
            specification=recipe.specification.model_dump(),
            manifest=artifact["object"],
        )
    document["policy"] = publish(
        extended.evidence, canonical_bytes(policy().model_dump(mode="json", by_alias=True))
    ).model_dump()
    return parse(document)


def calculate(value: GlobalWork, case: Case, output: Path) -> dict[str, object]:
    return prepare(
        value,
        evidence=case.evidence,
        views=tuple(case.view for _ in value.portfolios),
        primary_store=case.store,
        output=output,
    )


def test_registered_replay(extended: Case, tmp_path: Path) -> None:
    value = registered(extended, incremental=True)
    output = tmp_path / "global"
    output.mkdir(mode=0o700)
    artifact = calculate(value, extended, output)
    reference = CachedObject.model_validate(artifact["object"])
    document = json.loads(read_cached(output, reference))
    summary = json.loads(read_cached(output, CachedObject.model_validate(document["summary"])))
    assert summary["complete_matrix"] is True
    assert summary["distinct_strategies"] == 2
    assert summary["pbo"]["status"] == "available"
    assert all(item["dsr"]["status"] == "available" for item in summary["strategies"])
    clocks = {path.name: path.stat().st_mtime_ns for path in output.iterdir()}
    assert calculate(value.model_copy(update={"manifest": reference}), extended, output) == artifact
    assert clocks == {path.name: path.stat().st_mtime_ns for path in output.iterdir()}


@pytest.mark.parametrize("incomplete", [False, True])
def test_corrupt_primary(extended: Case, tmp_path: Path, incomplete: bool) -> None:
    value = registered(extended)
    if incomplete:
        document = value.model_dump(mode="json", by_alias=True)
        document["snapshot"]["states"][0]["state"] = 5
        value = parse(document)
    output = tmp_path / "global"
    output.mkdir(mode=0o700)
    path = extended.store / value.portfolios[0].manifest.sha256[7:]
    path.chmod(0o600)
    path.write_bytes(b"corrupt")
    with pytest.raises(ValueError):
        calculate(value, extended, output)
    assert not list(output.iterdir())
    assert path.read_bytes() == b"corrupt"


@pytest.mark.parametrize(
    ("field", "replacement"),
    [
        ("run_id", "run.changed"),
        ("factor_spec_id", "sha256:" + "44" * 32),
        ("specification_sha256", "sha256:" + "55" * 32),
        ("attempts", 2),
    ],
)
def test_historical_identity(tmp_path: Path, field: str, replacement: str | int) -> None:
    value = work()
    ledger = value.snapshot.ledger.model_dump(mode="json", by_alias=True)
    ledger["entries"][0][field] = replacement
    reference = publish(tmp_path, canonical_bytes({"trials": ledger}))
    source = value.portfolios[0].model_copy(update={"manifest": reference})
    with pytest.raises(ValueError, match="outside the current registry"):
        historical_ledger(source, value, tmp_path)


def test_changed_population(extended: Case, tmp_path: Path) -> None:
    value = registered(extended)
    output = tmp_path / "global"
    output.mkdir(mode=0o700)
    artifact = calculate(value, extended, output)
    reference = CachedObject.model_validate(artifact["object"])
    document = value.model_dump(mode="json", by_alias=True)
    document["snapshot"]["states"][0]["revision"] += 1
    document["snapshot"]["states"][0]["state"] = 5
    document["manifest"] = reference.model_dump()
    with pytest.raises((ValueError, FileNotFoundError)):
        calculate(parse(document), extended, output)
