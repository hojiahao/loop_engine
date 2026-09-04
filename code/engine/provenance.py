# -*- coding: utf-8 -*-
"""Evaluation provenance and stale-artifact detection."""
from __future__ import annotations

import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path

EVALUATION_SCHEMA_VERSION = 2


def operator_fingerprint() -> str:
    source = Path(__file__).with_name("operators.py").read_bytes()
    return hashlib.sha256(source).hexdigest()


def evaluator_fingerprint(evaluator) -> str:
    details = evaluator.provenance() if hasattr(evaluator, "provenance") else {
        "class": f"{type(evaluator).__module__}.{type(evaluator).__qualname__}"
    }
    encoded = json.dumps(details, sort_keys=True, ensure_ascii=True, default=str).encode()
    return hashlib.sha256(encoded).hexdigest()


def evaluation_record(evaluator) -> dict:
    return {
        "status": "current",
        "schema_version": EVALUATION_SCHEMA_VERSION,
        "operator_fingerprint": operator_fingerprint(),
        "evaluator_fingerprint": evaluator_fingerprint(evaluator),
        "evaluated_at": datetime.now(timezone.utc).isoformat(),
    }


def metrics_are_current(factor: dict) -> bool:
    record = factor.get("evaluation") or {}
    return (
        record.get("status") == "current"
        and record.get("schema_version") == EVALUATION_SCHEMA_VERSION
        and record.get("operator_fingerprint") == operator_fingerprint()
    )


def metrics_match_evaluator(factor: dict, evaluator) -> bool:
    record = factor.get("evaluation") or {}
    return metrics_are_current(factor) and (
        record.get("evaluator_fingerprint") == evaluator_fingerprint(evaluator)
    )


def stale_factor_hashes(factors: list[dict]) -> list[str]:
    return [str(f.get("hash", "<missing>")) for f in factors if not metrics_are_current(f)]
