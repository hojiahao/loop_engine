"""Immutable independent calculation, comparisons and byte-identical replay."""

import math
from pathlib import Path
from typing import Any, Literal

from loop_alphalens.artifacts import Deadline, Store, decode, encode, reference
from loop_alphalens.build import describe
from loop_alphalens.engine import calculate, columns, table
from loop_alphalens.models import Inputs, Receipt, Reference

ABSOLUTE_TOLERANCE = 1e-12
RELATIVE_TOLERANCE = 1e-10


def _numeric(value: str) -> float | None:
    if not value:
        return None
    result = float(value)
    if not math.isfinite(result):
        raise ValueError("comparison values must be finite or explicitly missing")
    return result


def compare(primary: bytes, independent: bytes, inputs: Inputs) -> list[dict[str, object]]:
    header = columns(inputs.groups)
    left, right = table(primary, header), table(independent, header)
    if len(left) != len(inputs.sessions) or len(right) != len(left):
        raise ValueError("comparison session count differs")
    differences: list[dict[str, object]] = []
    for date, actual, expected in zip(inputs.sessions, left, right, strict=True):
        for field in header:
            a, b = actual[field], expected[field]
            exact = field in header[:5]
            first, second = (None, None) if exact else (_numeric(a), _numeric(b))
            same = (
                a == b
                if exact
                else (
                    first is second
                    if first is None or second is None
                    else math.isclose(
                        first, second, rel_tol=RELATIVE_TOLERANCE, abs_tol=ABSOLUTE_TOLERANCE
                    )
                )
            )
            if not same:
                differences.append(
                    {
                        "session": date.isoformat(),
                        "field": field,
                        "primary": a,
                        "independent": b,
                        "absolute_tolerance": 0 if exact else ABSOLUTE_TOLERANCE,
                        "relative_tolerance": 0 if exact else RELATIVE_TOLERANCE,
                        "reason": "exact_field"
                        if exact
                        else (
                            "missingness"
                            if first is None or second is None
                            else "numerical_difference"
                        ),
                    }
                )
    return differences


def _lineage(inputs: Inputs, statistics: bytes, backtest: bytes) -> None:
    primary = decode(statistics)
    portfolio = decode(backtest)
    if (
        primary.get("schema") != "loop.statistics-receipt/v1"
        or not isinstance(primary.get("request"), dict)
        or primary["request"].get("backtest") != inputs.primary_backtest.model_dump()
        or primary.get("cross_sections") != inputs.primary_cross_sections.model_dump()
        or portfolio.get("schema") != "loop.portfolio-receipt/v1"
        or portfolio.get("quality") not in {"synthetic", "public_development"}
    ):
        raise ValueError("independent input lineage differs")
    for document in (primary, portfolio):
        if (
            document.get("source_code_sha256") != inputs.source_code_sha256
            or document.get("environment_sha256") != inputs.environment_sha256
            or document.get("production_eligible") is not False
        ):
            raise ValueError("independent provenance or data quality differs")


def run(
    store_path: Path, digest: str, *, replay: bool = False, seconds: float = 180
) -> dict[str, Any]:
    """Operate offline on one private CAS. No database or network permission.

    Rejected/unavailable comparisons are immutable diagnostics, not factor votes.
    Invalid bytes, bounds and dependency failures raise without a final receipt.
    Replay verifies every input/build/output and never writes or repairs files.
    """
    deadline = Deadline(seconds)
    store = Store(store_path)
    original: tuple[Reference, bytes] | None = None
    if replay:
        original = store.document(digest)
        decode(original[1])
        old = Receipt.model_validate_json(original[1])
        input_ref = old.inputs
        input_bytes = store.read(input_ref)
    else:
        input_ref, input_bytes = store.document(digest)
    decode(input_bytes)
    inputs = Inputs.model_validate_json(input_bytes)
    held: dict[str, tuple[Reference, bytes]] = {"input": (input_ref, input_bytes)}
    for name in (
        "primary_statistics",
        "primary_backtest",
        "observations",
        "primary_cross_sections",
    ):
        deadline.check()
        ref = getattr(inputs, name)
        held[name] = ref, store.read(ref)
    _lineage(inputs, held["primary_statistics"][1], held["primary_backtest"][1])
    build = describe(deadline)
    if replay and store.read(old.build) != build:
        raise ValueError("independent validator build changed")
    cross, turnover, available = calculate(inputs, held["observations"][1], deadline.check)
    differences = compare(held["primary_cross_sections"][1], cross, inputs)
    cross_rows = table(cross, columns(inputs.groups))
    missing = [row["decision_session"] for row in cross_rows[:-1] if row["status"] != "available"]
    disposition: Literal["accepted", "rejected", "unavailable"] = (
        "rejected"
        if differences
        else "unavailable"
        if missing or available < inputs.minimum_sessions
        else "accepted"
    )
    artifacts = {
        "build": build,
        "cross_sections": cross,
        "turnover": turnover,
        "differences": encode(
            {"schema": "loop.alphalens-differences/v1", "differences": differences}
        ),
        "summary": encode(
            {
                "schema": "loop.alphalens-summary/v1",
                "profile": inputs.profile,
                "disposition": disposition,
                "label": "next-session-raw-open-to-close.1",
                "absolute_tolerance": ABSOLUTE_TOLERANCE,
                "relative_tolerance": RELATIVE_TOLERANCE,
                "compared_sessions": len(inputs.sessions),
                "available_sessions": available,
                "minimum_sessions": inputs.minimum_sessions,
                "unavailable_sessions": missing,
                "differences": len(differences),
                "rank_ic": "Alphalens Spearman IC; no demeaning or group adjustment",
                "pearson_ic": "SciPy Pearson correlation",
                "groups": (
                    "Alphalens arithmetic group means; independent stable-ID equal-count bins"
                ),
                "turnover": (
                    "Alphalens quantile membership turnover; not executed-notional turnover"
                ),
                "shared_dependency": "frozen raw observations and primary export normalization",
                "zipline_accounting": "pending",
                "authorized_reconciliation": "pending",
                "production_eligible": False,
            }
        ),
    }
    if sum(map(len, artifacts.values())) > 64 * 1024 * 1024:
        raise ValueError("independent output byte budget")
    receipt = Receipt(
        inputs=input_ref,
        build=reference(artifacts["build"]),
        cross_sections=reference(artifacts["cross_sections"]),
        turnover=reference(artifacts["turnover"]),
        differences=reference(artifacts["differences"]),
        summary=reference(artifacts["summary"]),
        disposition=disposition,
    )
    encoded = encode(receipt.model_dump(mode="json", by_alias=True))
    if replay:
        if original is None or encoded != original[1]:
            raise ValueError("independent receipt differs from replay")
        for name, content in artifacts.items():
            deadline.check()
            if store.read(getattr(receipt, name)) != content:
                raise ValueError("independent output differs from replay")
    else:
        for content in artifacts.values():
            deadline.check()
            store.publish(content)
    for ref, content in held.values():
        deadline.check()
        if store.read(ref) != content:
            raise ValueError("independent input changed during calculation")
    if describe(deadline) != build:
        raise ValueError("independent build changed during calculation")
    deadline.check()
    if original is not None:
        if store.read(original[0]) != original[1]:
            raise ValueError("independent receipt changed during replay")
        result = original[0]
    else:
        result = store.publish(encoded)
    return {
        "receipt": result.model_dump(),
        "artifacts": receipt.model_dump(mode="json", by_alias=True),
    }
