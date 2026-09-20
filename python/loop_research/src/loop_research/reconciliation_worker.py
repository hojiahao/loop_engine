"""Export registered primary evidence for two independently supervised validators.

Only the Rust runtime authenticates jobs, leases and database registration. This
worker receives immutable references, never an executable or a caller verdict.
"""

import argparse
import sys
import time
from pathlib import Path
from typing import Literal

from pydantic import Field

from loop_research.alphalens_inputs import _export_alphalens
from loop_research.backtest import _Deadline, _paths
from loop_research.build_identity import canonical_bytes
from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import Identifier, ImmutableRecord
from loop_research.portfolio_worker import PortfolioWork, execute
from loop_research.statistics_models import StatisticsRequest
from loop_research.statistics_workflow import prepare_statistics, reconstruct_statistics
from loop_research.zipline_inputs import _export_zipline


class ValidationWork(ImmutableRecord):
    schema_version: Literal["loop.validation-work/v1"] = Field(alias="schema")
    job_id: Identifier
    lease_id: Identifier
    primary_revision: int = Field(ge=1, lt=2**63, strict=True)
    primary: PortfolioWork
    policy: CachedObject
    prepared: CachedObject | None = None


class PreparedInputs(ImmutableRecord):
    schema_version: Literal["loop.validation-inputs/v1"] = Field(
        default="loop.validation-inputs/v1", alias="schema"
    )
    job_id: Identifier
    lease_id: Identifier
    primary_job_id: Identifier
    primary_revision: int = Field(ge=1, lt=2**63, strict=True)
    primary_manifest: CachedObject
    policy: CachedObject
    statistics: CachedObject
    alphalens: CachedObject
    zipline: CachedObject
    production_eligible: Literal[False] = False


def prepare(
    work: ValidationWork, *, evidence: Path, view: Path, primary_store: Path, output: Path
) -> dict[str, object]:
    """Reconstruct before exporting, or verify all previous exports without writes.

    The supervisor additionally imposes a hard deadline across this process and
    both independent validators. The final authorized receipt is not made here.
    """
    deadline = _Deadline(180, time.monotonic)
    _paths(evidence, view, output)
    _paths(evidence, view, primary_store)
    if output == primary_store or work.primary.manifest is None:
        raise ValueError("separate validation output and registered primary required")
    if work.job_id == work.primary.job_id:
        raise ValueError("validation cannot name itself as primary")
    readonly = work.prepared is not None
    policy = read_cached(evidence, work.policy)
    decode_object(policy)
    # This verifies every original registered primary artifact, including its
    # statistical cross sections and global ledger, without repairing the store.
    execute(work.primary, evidence=evidence, view=view, output=primary_store)
    original = read_cached(primary_store, work.primary.manifest)
    document = decode_object(original)
    result = document["result"]
    supplementary = document["supplementary"]
    if not isinstance(result, dict) or not isinstance(supplementary, list):
        raise ValueError("primary portfolio envelope")
    artifacts = result.get("artifacts")
    if not isinstance(artifacts, dict):
        raise ValueError("primary portfolio artifacts")
    held: list[tuple[CachedObject, bytes]] = [(work.primary.manifest, original)]
    portfolio = None
    for item in [*artifacts.values(), *supplementary]:
        deadline.check()
        if not isinstance(item, dict) or not isinstance(item.get("schema"), dict):
            raise ValueError("primary artifact metadata")
        reference = CachedObject.model_validate(item["object"])
        content = read_cached(primary_store, reference)
        held.append((reference, content))
        publish(output, content, readonly=readonly)
        if item["schema"].get("name") == "loop.portfolio_receipt":
            if portfolio is not None:
                raise ValueError("ambiguous primary receipt")
            portfolio = reference
    if portfolio is None:
        raise ValueError("missing primary receipt")
    old = None
    if work.prepared is not None:
        old_bytes = read_cached(output, work.prepared)
        decode_object(old_bytes)
        old = PreparedInputs.model_validate_json(old_bytes)
        statistics, primary, guard = reconstruct_statistics(
            evidence, view, output, old.statistics.sha256, deadline
        )
        if statistics.artifacts.request != StatisticsRequest(backtest=portfolio):
            raise ValueError("validation statistics source differs")
    else:
        statistics, primary, guard = prepare_statistics(
            evidence, view, output, StatisticsRequest(backtest=portfolio), deadline
        )
    deadline.check()
    alpha = _export_alphalens(output, statistics, primary, guard, deadline, readonly=readonly)
    deadline.check()
    zipline = _export_zipline(output, portfolio, primary, deadline, readonly=readonly)
    prepared = PreparedInputs(
        job_id=work.job_id,
        lease_id=work.lease_id,
        primary_job_id=work.primary.job_id,
        primary_revision=work.primary_revision,
        primary_manifest=work.primary.manifest,
        policy=work.policy,
        statistics=statistics.receipt,
        alphalens=alpha,
        zipline=zipline,
    )
    if old is not None and old != prepared:
        raise ValueError("validation inputs differ from replay")
    for reference, content in held:
        deadline.check()
        if read_cached(primary_store, reference) != content:
            raise ValueError("primary evidence changed during export")
    if read_cached(evidence, work.policy) != policy:
        raise ValueError("comparison policy changed during export")
    guard()
    deadline.check()
    reference = publish(
        output, canonical_bytes(prepared.model_dump(mode="json", by_alias=True)), readonly=readonly
    )
    return {
        "reference": reference.model_dump(),
        "inputs": prepared.model_dump(mode="json", by_alias=True),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("evidence", "view", "primary-store", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    arguments = parser.parse_args()
    try:
        content = sys.stdin.buffer.read(1_048_577)
        if not 0 < len(content) <= 1_048_576:
            raise ValueError("validation work byte budget")
        decode_object(content)
        work = ValidationWork.model_validate_json(content)
        result = prepare(
            work,
            evidence=arguments.evidence,
            view=arguments.view,
            primary_store=arguments.primary_store,
            output=arguments.output,
        )
        sys.stdout.buffer.write(canonical_bytes(result))
        return 0
    except KeyboardInterrupt:
        return 130
    except OSError, ValueError, TimeoutError, KeyError, TypeError:
        print("validation export refused execution", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
