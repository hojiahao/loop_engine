"""Causal OHLCV construction for the existing authorized development worker."""

import csv
import hashlib
import io
import math
import os
import time
from collections.abc import Callable
from datetime import UTC, date, datetime, timedelta
from importlib.metadata import version
from pathlib import Path

from loop_research.build_identity import canonical_bytes, describe_source
from loop_research.calendar import require_session_decisions, xnys_session_dates
from loop_research.data.fetch_cache import (
    private_directory,
    publish,
    read_cached,
    read_config_bytes,
    read_receipt,
)
from loop_research.data.fetch_json import decode_object
from loop_research.data.fetch_records import CachedObject
from loop_research.data.models import PitInput, PitQuery, RawBar
from loop_research.data.query import security_states, universe_eligible
from loop_research.evaluator import MAX_CELLS, MAX_WORK
from loop_research.panel_io import MAX_METADATA_BYTES, MAX_PANEL_BYTES, ContentRef, PanelManifest
from loop_research.panel_models import PanelReceipt, PanelReport, PanelRequest, RawField
from loop_research.panel_sources import load_capture
from loop_research.transform_models import PanelTransform


class _Budget:
    def __init__(self, seconds: float, clock: Callable[[], float]) -> None:
        if not math.isfinite(seconds) or not 0 < seconds <= 180:
            raise ValueError("panel construction deadline budget")
        self.clock = clock
        self.previous = self.started = clock()
        self.seconds = seconds
        self.used = 0
        self.check()

    def check(self) -> float:
        current = self.clock()
        if (
            not math.isfinite(current)
            or not math.isfinite(self.previous)
            or current < self.previous
        ):
            raise ValueError("panel construction clock regression")
        self.previous = current
        remaining = self.seconds - (current - self.started)
        if remaining <= 0:
            raise ValueError("panel construction deadline exceeded")
        return remaining

    def charge(self, units: int) -> None:
        self.used += units
        if self.used > MAX_WORK:
            raise ValueError("panel construction work budget")
        self.check()


def load_panel_request(path: Path) -> PanelRequest:
    """Read bounded unambiguous JSON; paths and requests are administrative input."""
    content = read_config_bytes(path)
    decode_object(content)
    return PanelRequest.model_validate_json(content)


def _milliseconds(instant: datetime) -> int:
    delta = instant.astimezone(UTC) - datetime(1970, 1, 1, tzinfo=UTC)
    # Round knowledge forward, never into an earlier millisecond.
    return (delta.days * 86400 + delta.seconds) * 1000 + (delta.microseconds + 999) // 1000


def _schedule(
    request: PanelRequest,
) -> tuple[tuple[date, ...], tuple[datetime, ...], tuple[datetime, ...], tuple[int, ...]]:
    import exchange_calendars as xcals  # type: ignore[import-untyped]

    sessions = xnys_session_dates(request.warmup_start, request.sample_end)
    expected = xnys_session_dates(request.sample_start, request.sample_end)
    if not expected or not sessions or len(sessions) * len(request.securities) > MAX_CELLS:
        raise ValueError("panel selection has no evaluation session or exceeds cell budget")
    calendar = xcals.get_calendar(
        "XNYS",
        start=(sessions[0] - timedelta(days=7)).isoformat(),
        end=(sessions[-1] + timedelta(days=7)).isoformat(),
    )
    opens = tuple(calendar.session_open(day.isoformat()).to_pydatetime() for day in sessions)
    closes = tuple(calendar.session_close(day.isoformat()).to_pydatetime() for day in sessions)
    decisions = tuple(_milliseconds(close) + request.close_delay_ms for close in closes)
    require_session_decisions(sessions, decisions)
    return sessions, opens, closes, decisions


def _number(bar: RawBar, field: RawField) -> str:
    original = getattr(bar, field.removeprefix("market."))
    value = float(original)
    if not math.isfinite(value) or (field == "market.volume" and int(value) != original):
        raise ValueError("selected panel value loses integer precision or is non-finite")
    return format(value, ".17g")


def _rows(
    capture: PitInput,
    request: PanelRequest,
    schedule: tuple[tuple[date, ...], tuple[datetime, ...], tuple[datetime, ...], tuple[int, ...]],
    budget: _Budget,
) -> tuple[bytes, tuple[int, int, int, int]]:
    sessions, opens, closes, decisions = schedule
    last_decision = datetime(1970, 1, 1, tzinfo=UTC) + timedelta(milliseconds=decisions[-1])
    if capture.captured_at < last_decision:
        raise ValueError("capture precedes the last decision")
    indexed: dict[tuple[str, date], list[RawBar]] = {}
    selected = set(request.securities)
    for record in capture.bars:
        if (
            record.security_id in selected
            and request.warmup_start <= record.session <= request.sample_end
        ):
            indexed.setdefault((record.security_id, record.session), []).append(record)
    stream = io.StringIO(newline="")
    writer = csv.writer(stream, lineterminator="\n")
    writer.writerow(["session", "security_id", "eligible", "known_at_ms", *request.fields])
    eligible_count = observed_count = evaluation_eligible = evaluation_observed = 0
    for day, opening, closing, decision_ms in zip(sessions, opens, closes, decisions, strict=True):
        budget.charge(len(capture.securities) + len(capture.bars) + len(request.securities))
        decision = datetime(1970, 1, 1, tzinfo=UTC) + timedelta(milliseconds=decision_ms)
        query = PitQuery(market_at=decision, known_at=decision, ingested_at=capture.captured_at)
        states = {state.security_id: state for state in security_states(capture, query)}
        for security in request.securities:
            state = states.get(security)
            eligible = state is not None and universe_eligible(state)
            bar = max(
                (
                    record
                    for record in indexed.get((security, day), ())
                    if eligible
                    and record.effective_at <= decision
                    and record.known_at <= decision
                    and record.ingested_at <= capture.captured_at
                ),
                key=lambda record: record.known_at,
                default=None,
            )
            if bar is not None and (
                bar.currency != "USD" or bar.interval_start > opening or bar.effective_at < closing
            ):
                raise ValueError("selected bar must cover the XNYS session in raw USD")
            known = state.known_at if state else None
            if bar is not None:
                known = max(known or bar.known_at, bar.known_at)
            writer.writerow(
                [
                    day.isoformat(),
                    security,
                    "1" if eligible else "0",
                    str(_milliseconds(known)) if known else "",
                    *(_number(bar, field) if bar else "" for field in request.fields),
                ]
            )
            eligible_count += int(eligible)
            observed_count += int(bar is not None)
            if day >= request.sample_start:
                evaluation_eligible += int(eligible)
                evaluation_observed += int(bar is not None)
        if stream.tell() > MAX_PANEL_BYTES:
            raise ValueError("panel CSV byte budget")
    return stream.getvalue().encode("ascii"), (
        eligible_count,
        observed_count,
        evaluation_eligible,
        evaluation_observed,
    )


def _materialize(
    sources: Path,
    request: PanelRequest,
    budget: _Budget,
) -> tuple[PanelReceipt, dict[str, tuple[CachedObject, bytes]]]:
    request = PanelRequest.model_validate(request)
    source = describe_source()["sha256"]
    capture = load_capture(sources, request, budget.check)
    schedule = _schedule(request)
    csv_bytes, counts = _rows(capture, request, schedule, budget)
    objects: dict[str, tuple[CachedObject, bytes]] = {}

    def object_ref(content: bytes) -> CachedObject:
        reference = CachedObject(
            sha256="sha256:" + hashlib.sha256(content).hexdigest(), byte_size=len(content)
        )
        if objects.setdefault(reference.sha256, (reference, content)) != (reference, content):
            raise ValueError("conflicting derived object identity")
        return reference

    def document(value: object) -> CachedObject:
        content = canonical_bytes(value)
        if len(content) > MAX_METADATA_BYTES:
            raise ValueError("panel metadata byte budget")
        return object_ref(content)

    def artifact(
        reference: CachedObject,
        name: str,
        media: str,
        columns: list[str],
        *,
        schema_version: int = 1,
    ) -> dict[str, object]:
        schema = document(
            {
                "schema": "loop.artifact-schema/v1",
                "name": name,
                "version": schema_version,
                "media_type": media,
                "columns": columns,
            }
        )
        return {
            "object": reference.model_dump(),
            "schema": {"name": name, "version": schema_version, "document": schema.model_dump()},
            "media_type": media,
            "created_at_ms": _milliseconds(capture.captured_at),
        }

    sessions, _, _, decisions = schedule
    calendar_version = version("exchange-calendars")
    values = object_ref(csv_bytes)
    transformation = None
    exposure_artifacts: list[dict[str, object]] = []
    if request.transform is not None:
        from loop_research.exposure_io import COLUMNS, prepare_exposures

        recipe, exposure_bytes = prepare_exposures(
            sources,
            request.transform,
            quality=capture.quality,
            captured_at=capture.captured_at,
            sessions=sessions,
            securities=request.securities,
            decisions=decisions,
            check=budget.check,
            charge=budget.charge,
        )
        exposure_ref = object_ref(exposure_bytes) if exposure_bytes is not None else None
        transformation = PanelTransform(
            preprocess=recipe.preprocess,
            neutralization=recipe.neutralization,
            exposures=exposure_ref,
        )
        if exposure_ref is not None:
            exposure_artifacts.append(
                artifact(exposure_ref, "loop.factor_exposures", "text/csv", COLUMNS)
            )
    panel = PanelManifest(
        schema="loop.factor-panel/v1" if transformation is None else "loop.factor-panel/v2",
        quality=capture.quality,
        sessions=tuple(day.isoformat() for day in sessions),
        securities=request.securities,
        fields=request.fields,
        decision_times_ms=decisions,
        evaluation_start=next(day.isoformat() for day in sessions if day >= request.sample_start),
        values=ContentRef(**values.model_dump()),
        transform=transformation,
    )
    panel_ref = object_ref(panel.model_dump_json(by_alias=True).encode("ascii"))
    calendar = document(
        {
            "schema": "loop.trading-calendar/v1",
            "name": "XNYS",
            "timezone": "America/New_York",
            "sessions": list(panel.sessions),
        }
    )
    identity = (
        "panel."
        + hashlib.sha256(
            canonical_bytes(
                {
                    "request": request.model_dump(mode="json", by_alias=True),
                    "builder": "causal-raw-ohlcv.1",
                    "source_code_sha256": source,
                    "calendar_version": calendar_version,
                }
            )
        ).hexdigest()
    )
    dataset = document(
        {
            "schema": "loop.development-dataset/v1",
            "sample": {
                "role": "in_sample"
                if request.sample_end.year <= 2016
                else "development_validation",
                "start": request.sample_start.isoformat(),
                "end": request.sample_end.isoformat(),
            },
            "quality": capture.quality,
            "snapshots": [
                {
                    "snapshot_id": identity,
                    "source": "causal-raw-ohlcv.1",
                    "dataset": "factor-panel",
                    "entitlement": "offline-synthetic"
                    if capture.quality == "synthetic"
                    else "public-development",
                    "known_through_ms": decisions[-1],
                    "artifacts": [
                        artifact(
                            panel_ref,
                            "loop.factor_panel",
                            "application/json",
                            [],
                            schema_version=1 if transformation is None else 2,
                        ),
                        artifact(
                            values,
                            "loop.factor_panel_values",
                            "text/csv",
                            [
                                "session",
                                "security_id",
                                "eligible",
                                "known_at_ms",
                                *request.fields,
                            ],
                        ),
                        *exposure_artifacts,
                    ],
                }
            ],
        }
    )
    budget.check()
    if source != describe_source()["sha256"]:
        raise ValueError("panel builder source changed during construction")
    receipt = PanelReceipt(
        request=request,
        source_code_sha256=source,
        calendar_version=calendar_version,
        quality=capture.quality,
        panel=panel_ref,
        values=values,
        calendar=calendar,
        dataset=dataset,
        rows=len(sessions) * len(request.securities),
        eligible_rows=counts[0],
        observed_rows=counts[1],
        evaluation_eligible_rows=counts[2],
        evaluation_observed_rows=counts[3],
    )
    return receipt, objects


def _stores(sources: Path, output: Path) -> None:
    for directory in (sources, output):
        os.close(private_directory(directory))
    if sources == output or sources.is_relative_to(output) or output.is_relative_to(sources):
        raise ValueError("private source and development output stores must be separate")


def _report(reference: CachedObject, receipt: PanelReceipt) -> PanelReport:
    return PanelReport(
        receipt=reference,
        panel=receipt.panel,
        calendar=receipt.calendar,
        dataset=receipt.dataset,
        quality=receipt.quality,
        rows=receipt.rows,
        eligible_rows=receipt.eligible_rows,
        observed_rows=receipt.observed_rows,
        evaluation_eligible_rows=receipt.evaluation_eligible_rows,
        evaluation_observed_rows=receipt.evaluation_observed_rows,
    )


def build_panel(
    sources: Path,
    output: Path,
    request: PanelRequest,
    *,
    timeout_seconds: float = 180,
    monotonic: Callable[[], float] = time.monotonic,
) -> PanelReport:
    """Publish derived artifacts, then a final private construction receipt.

    The local data owner supplies separate stores. No registration, numerical
    worker, network or holdout capability is invoked. Cancellation/IO failure
    may retain intermediate objects; only the final receipt signals completion.
    Existing objects are verified, never overwritten. Retry is deterministic.
    """
    _stores(sources, output)
    budget = _Budget(timeout_seconds, monotonic)
    receipt, objects = _materialize(sources, request, budget)
    content = receipt.model_dump_json(by_alias=True).encode("ascii")
    if len(content) > 128 * 1024:
        raise ValueError("panel receipt byte budget")
    for reference, payload in objects.values():
        budget.check()
        if publish(output, payload) != reference:
            raise ValueError("panel publication identity differs")
    budget.check()
    return _report(publish(sources, content), receipt)


def validate_panel(
    sources: Path,
    output: Path,
    digest: str,
    *,
    timeout_seconds: float = 180,
    monotonic: Callable[[], float] = time.monotonic,
) -> PanelReport:
    """Rebuild input/lineage and verify every derived byte without publishing.

    Missing, changed, unsupported or corrupt evidence fails closed. A successful
    report remains a local data-owner check, not a reusable runtime permission.
    """
    _stores(sources, output)
    budget = _Budget(timeout_seconds, monotonic)
    reference, content = read_receipt(sources, digest)
    stored = PanelReceipt.model_validate_json(content)
    if stored.model_dump_json(by_alias=True).encode("ascii") != content:
        raise ValueError("panel receipt must be canonical JSON")
    receipt, objects = _materialize(sources, stored.request, budget)
    if receipt != stored:
        raise ValueError("panel construction provenance or output differs")
    for expected, payload in objects.values():
        budget.check()
        if read_cached(output, expected) != payload:
            raise ValueError("derived panel bytes differ")
    return _report(reference, receipt)
