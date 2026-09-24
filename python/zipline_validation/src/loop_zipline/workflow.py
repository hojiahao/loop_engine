"""Immutable independent reconciliation, with raw Zipline and economic NAV evidence."""

import csv
import io
from collections.abc import Callable
from fractions import Fraction
from pathlib import Path
from typing import Any, Literal

from loop_zipline.accounting import Tables
from loop_zipline.artifacts import Deadline, Store, decode, encode, reference
from loop_zipline.build import describe
from loop_zipline.engine import calculate
from loop_zipline.models import Artifacts, Inputs, Policy, Receipt, Reference
from loop_zipline.native import Unavailable


def table(
    content: bytes, header: str, check: Callable[[], None] = lambda: None
) -> list[dict[str, str]]:
    reader = csv.DictReader(io.StringIO(content.decode("ascii"), newline=""), strict=True)
    if reader.fieldnames != header.split(","):
        raise ValueError("independent comparison schema differs")
    rows: list[dict[str, str]] = []
    for row in reader:
        check()
        if None in row or None in row.values() or len(rows) >= 400_000:
            raise ValueError("independent comparison row bound or width")
        rows.append(row)
    return rows


def compare(
    primary: dict[str, bytes],
    actual: dict[str, bytes],
    market: bool,
    check: Callable[[], None] = lambda: None,
) -> tuple[int, list[dict[str, object]]]:
    differences: list[dict[str, object]] = []
    count = 0
    for name, header in Tables(market).headers.items():
        left, right = table(primary[name], header, check), table(actual[name], header, check)
        if len(left) != len(right):
            count += 1
            if len(differences) < 10_000:
                differences.append(
                    {
                        "artifact": name,
                        "field": "row_count",
                        "primary": len(left),
                        "independent": len(right),
                    }
                )
        for index, (expected, observed) in enumerate(zip(left, right, strict=False)):
            check()
            for field in header.split(","):
                first, second = expected[field], observed[field]
                tolerance = (
                    5e-9
                    if field in {"price_usd", "mark_usd"}
                    else 1e-12
                    if field == "simple_return"
                    else 1e-5
                    if field.endswith("_usd")
                    else 0.0
                )
                same = first == second
                if tolerance and first and second:
                    # Parse the printed decimal exactly: converting both sides
                    # to float could conceal a one-tick difference at high prices.
                    same = abs(Fraction(first) - Fraction(second)) <= Fraction(str(tolerance))
                if not same:
                    count += 1
                    if len(differences) < 10_000:
                        differences.append(
                            {
                                "artifact": name,
                                "row": index,
                                "field": field,
                                "primary": first,
                                "independent": second,
                                "absolute_tolerance": tolerance,
                                "reason": "missingness"
                                if not first or not second
                                else "numerical_difference"
                                if tolerance
                                else "exact_field",
                            }
                        )
    return count, differences


def lineage(inputs: Inputs, content: bytes) -> None:
    document = decode(content)
    if (
        document.get("schema") != "loop.portfolio-receipt/v1"
        or document.get("engine") != inputs.engine
        or document.get("quality") not in {"synthetic", "public_development"}
    ):
        raise ValueError("independent portfolio lineage differs")
    if (
        document.get("production_eligible") is not False
        or document.get("artifacts") != inputs.primary_artifacts.model_dump()
    ):
        raise ValueError("independent portfolio references differ")
    for name in ("source_code_sha256", "environment_sha256"):
        if document.get(name) != getattr(inputs, name):
            raise ValueError("independent provenance differs")
    request = document.get("request")
    if not isinstance(request, dict) or not isinstance(request.get("policies"), dict):
        raise ValueError("missing frozen portfolio policies")
    policies = request["policies"]
    algorithms = (
        ("ranked-long-short.1", "pit-next-open.1", "commission-impact-finance.1")
        if inputs.engine == "pit-actions-long-short.1"
        else ("long-only-top-n.1", "next-session-open.1", "commission-spread.1")
    )
    parameters: dict[str, Any] = {}
    for role, algorithm in zip(
        ("portfolio_policy", "execution_policy", "cost_policy"), algorithms, strict=True
    ):
        record = policies.get(role)
        if not isinstance(record, dict) or not isinstance(record.get("settings"), dict):
            raise ValueError("missing frozen accounting policy")
        values = record["settings"]
        if values.get("algorithm") != algorithm:
            raise ValueError("unsupported independent accounting policy")
        for key, value in values.items():
            if key == "algorithm":
                continue
            if key in parameters or not isinstance(value, str):
                raise ValueError("ambiguous independent policy parameter")
            parameters[key] = value if key.endswith("_usd") else int(value)
    if Policy.model_validate(parameters) != inputs.policy:
        raise ValueError("independent parameters differ from the frozen portfolio")


def run(
    store_path: Path, digest: str, *, replay: bool = False, seconds: float = 180
) -> dict[str, Any]:
    """Offline diagnostic only. Invalid authority/data never becomes a factor vote.

    Write immutable artifacts before the final receipt. On replay, re-read every
    input, recalculate and byte-verify every output without writes or repair.
    """
    deadline = Deadline(seconds)
    store = Store(store_path)
    original: tuple[Reference, bytes] | None = None
    old: Receipt | None = None
    if replay:
        original = store.document(digest)
        decode(original[1])
        old = Receipt.model_validate_json(original[1])
        input_ref, input_bytes = old.inputs, store.read(old.inputs)
    else:
        input_ref, input_bytes = store.document(digest)
    decode(input_bytes)
    inputs = Inputs.model_validate_json(input_bytes)
    held = {"inputs": (input_ref, input_bytes)}
    for name in ("primary_backtest", "observations"):
        ref = getattr(inputs, name)
        held[name] = (ref, store.read(ref))
    lineage(inputs, held["primary_backtest"][1])
    primary = {}
    for name in Artifacts.model_fields:
        ref = getattr(inputs.primary_artifacts, name)
        content = store.read(ref)
        held[name] = ref, content
        primary[name] = content
    build = describe(deadline)
    if old is not None and store.read(old.build) != build:
        raise ValueError("independent accounting build changed")
    decode(held["observations"][1])
    reason = None
    try:
        ledgers, bridge = calculate(inputs, held["observations"][1], deadline.check)
    except Unavailable as error:
        reason = str(error)
        ledgers = Tables(inputs.engine == "pit-actions-long-short.1").finish()
        bridge = encode({"schema": "loop.zipline-bridge/v1", "sessions": []})
    count, differences = (
        compare(primary, ledgers, inputs.engine == "pit-actions-long-short.1", deadline.check)
        if reason is None
        else (0, [])
    )
    disposition: Literal["accepted", "rejected", "unavailable"] = (
        "unavailable" if reason else "rejected" if count else "accepted"
    )
    extras = {
        "build": build,
        "bridge": bridge,
        "differences": encode(
            {
                "schema": "loop.zipline-differences/v1",
                "total": count,
                "omitted": count - len(differences),
                "differences": differences,
            }
        ),
        "summary": encode(
            {
                "schema": "loop.zipline-summary/v1",
                "profile": inputs.profile,
                "engine": inputs.engine,
                "disposition": disposition,
                "reason": reason,
                "differences": count,
                "dollar_tolerance": 1e-5,
                "price_tolerance": 5e-9,
                "return_tolerance": 1e-12,
                "relative_tolerance": 0,
                "execution": "Zipline SimulationBlotter at observed opening events",
                "accounting": (
                    "Zipline Ledger transaction cash, commissions, positions and native NAV"
                ),
                "extensions": [
                    "rational sizing and dated risk constraints",
                    "signed unpaid action claims in economic NAV",
                    "explicit rational whole-share split and delayed cash-in-lieu",
                    "declared cash delisting and internal financing flows",
                ],
                "shared_dependency": "frozen raw inputs and primary PIT export normalization",
                "authorized_reconciliation": "pending",
                "production_eligible": False,
            }
        ),
    }
    if sum(map(len, (*ledgers.values(), *extras.values()))) > 64 * 1024 * 1024:
        raise ValueError("independent result byte bound")
    receipt = Receipt(
        inputs=input_ref,
        ledgers=Artifacts(**{name: reference(content) for name, content in ledgers.items()}),
        disposition=disposition,
        build=reference(extras["build"]),
        bridge=reference(extras["bridge"]),
        differences=reference(extras["differences"]),
        summary=reference(extras["summary"]),
    )
    encoded = encode(receipt.model_dump(mode="json", by_alias=True))
    if original is not None and encoded != original[1]:
        raise ValueError("independent receipt differs from replay")
    outputs = [(getattr(receipt.ledgers, name), content) for name, content in ledgers.items()]
    outputs.extend((getattr(receipt, name), content) for name, content in extras.items())
    for ref, content in outputs:
        deadline.check()
        if old is not None:
            if store.read(ref) != content:
                raise ValueError("independent output differs from replay")
        else:
            store.publish(content)
    for ref, content in held.values():
        deadline.check()
        if store.read(ref) != content:
            raise ValueError("independent input changed during execution")
    if describe(deadline) != build:
        raise ValueError("independent build changed during execution")
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
