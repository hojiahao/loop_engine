"""Deterministic historical queries; later invisible versions never win."""

from collections.abc import Callable, Hashable, Iterable
from typing import Literal

from pydantic import Field

from loop_research.data.models import (
    Fundamental,
    ImmutableRecord,
    PitInput,
    PitQuery,
    RawBar,
    SecurityVersion,
    TemporalRecord,
    fact_key,
)


def _visible(record: TemporalRecord, query: PitQuery) -> bool:
    return record.known_at <= query.known_at and record.ingested_at <= query.ingested_at


def _latest[T: TemporalRecord, K: Hashable](
    records: Iterable[T], key: Callable[[T], K]
) -> dict[K, T]:
    result: dict[K, T] = {}
    for record in records:
        identity = key(record)
        previous = result.get(identity)
        if previous is None or record.known_at > previous.known_at:
            result[identity] = record
    return result


def universe_eligible(record: SecurityVersion) -> bool:
    """Default ordinary-common-stock rule, independent of current ticker lists."""
    return (
        record.listed and record.kind == "common_stock" and record.venue in {"XNYS", "XNAS", "XASE"}
    )


def security_states(capture: PitInput, query: PitQuery) -> tuple[SecurityVersion, ...]:
    """Resolve visible effective states, then expire them without falling back.

    Latest-public revisions at each effective instant replace older revisions.
    A later effective event then supersedes all earlier events for that security.
    Explicitly delisted states are retained for ID lookup; expired intervals are
    absent. Conflicting visible listed ticker assignments raise ValueError.
    """
    if query.ingested_at > capture.captured_at:
        raise ValueError("query ingestion cutoff exceeds the captured input")
    revisions = _latest(
        (
            record
            for record in capture.securities
            if record.effective_at <= query.market_at and _visible(record, query)
        ),
        lambda record: (record.security_id, record.effective_at),
    )
    states: dict[str, SecurityVersion] = {}
    for record in revisions.values():
        previous = states.get(record.security_id)
        if previous is None or record.effective_at > previous.effective_at:
            states[record.security_id] = record
    valid = tuple(
        record
        for _, record in sorted(states.items())
        if record.effective_until is None or query.market_at < record.effective_until
    )
    aliases: set[tuple[str, str]] = set()
    for record in valid:
        if record.listed:
            alias = record.venue, record.ticker
            if alias in aliases:
                raise ValueError("ambiguous listed ticker at the requested decision")
            aliases.add(alias)
    return valid


class SelectedSecurity(ImmutableRecord):
    """Resolved state and default universe eligibility, not a trading permission."""

    state: SecurityVersion
    universe_eligible: bool


class PitResult(ImmutableRecord):
    """Historical selections from one capture; missing records are never filled."""

    schema_version: Literal["loop.pit-query-result/v1"] = Field(
        default="loop.pit-query-result/v1", alias="schema"
    )
    query: PitQuery
    securities: tuple[SelectedSecurity, ...]
    bars: tuple[RawBar, ...]
    fundamentals: tuple[Fundamental, ...]


def query_capture(capture: PitInput, query: PitQuery) -> PitResult:
    """Query bounded immutable input with explicit business/public/ingestion times.

    Returns only matching visible versions and their source evidence. A ticker
    must be listed at the market time; an ID lookup may return a delisted state.
    Bars and facts are filtered to selected security/issuer identities, but are
    not implicitly joined, adjusted, resampled, neutralized or forward-filled.
    """
    capture = PitInput.model_validate(capture)
    query = PitQuery.model_validate(query)
    states = security_states(capture, query)
    if query.security_id is not None:
        selected = tuple(record for record in states if record.security_id == query.security_id)
    elif query.ticker is not None:
        selected = tuple(
            record
            for record in states
            if record.listed and (record.venue, record.ticker) == (query.venue, query.ticker)
        )
    else:
        selected = tuple(record for record in states if universe_eligible(record))
    security_ids = {record.security_id for record in selected}
    issuer_ids = {record.issuer_id for record in selected}
    bars = _latest(
        (record for record in capture.bars if _visible(record, query)),
        lambda record: (record.security_id, record.session),
    )
    facts = _latest(
        (record for record in capture.fundamentals if _visible(record, query)), fact_key
    )
    return PitResult(
        query=query,
        securities=tuple(
            SelectedSecurity(state=record, universe_eligible=universe_eligible(record))
            for record in selected
        ),
        bars=tuple(
            record
            for _, record in sorted(bars.items())
            if record.security_id in security_ids and record.effective_at <= query.market_at
        ),
        fundamentals=tuple(
            sorted(
                (
                    record
                    for record in facts.values()
                    if record.issuer_id in issuer_ids and record.effective_at <= query.market_at
                ),
                key=lambda record: (
                    record.issuer_id,
                    record.concept,
                    record.unit,
                    record.period_start.isoformat() if record.period_start else "",
                    record.period_end,
                ),
            )
        ),
    )
