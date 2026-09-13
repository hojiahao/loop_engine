"""SEC company-facts normalization with conservative first-observed knowledge."""

import hashlib
import re
from datetime import UTC, date, datetime, time
from decimal import Decimal
from typing import Any

from loop_research.data.fetch_config import SecRequest
from loop_research.data.fetch_http import Download, FetchError
from loop_research.data.fetch_json import decimal_text, decode_object
from loop_research.data.fetch_records import DevelopmentBatch
from loop_research.data.models import Fundamental, SourceEvidence, fact_key


def _cik(value: object) -> str:
    if type(value) is int:
        value = str(value)
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{1,10}", value) or int(value) == 0:
        raise FetchError("identity_unresolved")
    return value.zfill(10)


def _date(value: object) -> date:
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value):
        raise ValueError("SEC period requires a date")
    return date.fromisoformat(value)


def normalize_sec(
    config: SecRequest,
    facts: Download,
    submissions: Download,
) -> DevelopmentBatch:
    """Resolve current observed values, never reconstruct historical availability.

    Filing dates only order vintages within this current API response. Same-date
    conflicting values are ambiguous and fail closed; all vintages remain in
    the original cached response. Units and instant/duration periods stay separate.
    Raises ValueError for malformed observations and FetchError for bounds/identity.
    """
    payload, metadata = decode_object(facts.body), decode_object(submissions.body)
    if _cik(payload.get("cik")) != config.cik or _cik(metadata.get("cik")) != config.cik:
        raise FetchError("identity_unresolved")
    namespaces = payload.get("facts")
    if not isinstance(namespaces, dict) or not isinstance(metadata.get("filings"), dict):
        raise ValueError("missing SEC facts or filing metadata")
    raw_hash = "sha256:" + hashlib.sha256(facts.body).hexdigest()
    selected: dict[tuple[str, str, str, date | None, date], tuple[date, Fundamental]] = {}
    vintages: dict[tuple[str, str, str, date | None, date, date], Decimal] = {}
    examined = 0
    for concept in config.concepts:
        namespace, name = concept.split(":")
        taxonomy = namespaces.get(namespace, {})
        if not isinstance(taxonomy, dict):
            raise ValueError("invalid SEC taxonomy")
        definition = taxonomy.get(name)
        if definition is None:
            continue
        if not isinstance(definition, dict) or not isinstance(definition.get("units"), dict):
            raise ValueError("missing SEC units")
        for unit, rows in definition["units"].items():
            if not isinstance(rows, list):
                raise ValueError("SEC observations require an array")
            for row in rows:
                if not isinstance(row, dict):
                    raise ValueError("invalid SEC observation")
                period_end = _date(row.get("end"))
                if not config.start <= period_end <= config.end:
                    continue
                examined += 1
                if examined > config.budget.records:
                    raise FetchError("record_budget")
                filed = _date(row.get("filed"))
                if not period_end <= filed <= facts.observed_at.date():
                    raise ValueError("invalid SEC filing chronology")
                fact = _fact(row, config, concept, unit, period_end, facts, raw_hash)
                identity = fact_key(fact)
                vintage = (*identity, filed)
                numeric_value = Decimal(fact.value)
                if vintages.setdefault(vintage, numeric_value) != numeric_value:
                    raise ValueError("conflicting same-date SEC vintages")
                previous = selected.get(identity)
                if previous is None or (filed, fact.filing_id, fact.value) > (
                    previous[0],
                    previous[1].filing_id,
                    previous[1].value,
                ):
                    selected[identity] = filed, fact
    records = tuple(
        sorted(
            (fact for _, fact in selected.values()),
            key=lambda fact: (
                fact.concept,
                fact.unit,
                fact.period_start or date.min,
                fact.period_end,
            ),
        )
    )
    found = {fact.concept for fact in records}
    return DevelopmentBatch(
        provider="sec",
        start=config.start,
        end=config.end,
        fundamentals=records,
        missing=tuple(sorted(set(config.concepts) - found)),
    )


def _fact(
    row: dict[str, Any],
    config: SecRequest,
    concept: str,
    unit: str,
    period_end: date,
    download: Download,
    raw_hash: str,
) -> Fundamental:
    accession = row.get("accn")
    if not isinstance(accession, str) or not re.fullmatch(
        r"[0-9]{10}-[0-9]{2}-[0-9]{6}", accession
    ):
        raise ValueError("invalid SEC accession")
    period_start = _date(row["start"]) if "start" in row else None
    value = decimal_text(row.get("val"))
    identity = ":".join((config.cik, concept, unit, str(period_start), str(period_end), accession))
    return Fundamental(
        issuer_id="sec:cik:" + config.cik,
        concept=concept,
        unit=unit,
        period_start=period_start,
        period_end=period_end,
        filing_id=accession,
        value=value,
        effective_at=datetime.combine(period_end, time.max, UTC),
        known_at=download.observed_at,
        ingested_at=download.observed_at,
        source=SourceEvidence(
            source="sec",
            dataset="companyfacts",
            revision=raw_hash,
            record_id="sec:fact:" + hashlib.sha256(identity.encode()).hexdigest(),
            raw_sha256=raw_hash,
            availability="first_observed",
        ),
    )
