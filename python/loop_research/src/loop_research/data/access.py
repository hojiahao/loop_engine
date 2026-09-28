"""Offline access diagnostics shared with the real source synchronization gate."""

import hashlib
import json
import os
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Literal
from zoneinfo import ZoneInfo

from pydantic import Field, TypeAdapter

from loop_research.data.fetch_cache import (
    private_directory,
    read_config_bytes,
    read_private_config,
)
from loop_research.data.fetch_config import AlpacaRequest, SecRequest, SecretReference
from loop_research.data.fetch_http import FailureReason, FetchError
from loop_research.data.fetch_json import contains_secret
from loop_research.data.ingestion import _headers, _toml_date
from loop_research.data.licensed_config import WrdsRequest, requested_datasets
from loop_research.data.licensed_ingestion import _credentials, _license
from loop_research.data.models import ImmutableRecord, Instant, parse_instant
from loop_research.data.snapshot_models import SourceRequest, SyncPlan

SOURCE_CONFIG: TypeAdapter[SourceRequest] = TypeAdapter(SourceRequest)


@dataclass(frozen=True, slots=True)
class LicenseFile:
    """Bounded owner-only bytes; never a supplier signature or a reusable grant."""

    path: Path = field(repr=False)
    content: bytes = field(repr=False)


class SourceAccess(ImmutableRecord):
    """Only non-secret references and static failure reasons leave preflight."""

    request_index: int = Field(ge=0, lt=32)
    provider: Literal["sec", "alpaca", "sharadar", "wrds", "databento"]
    credential_references: tuple[SecretReference, ...]
    missing_references: tuple[SecretReference, ...]
    license_datasets: tuple[str, ...]
    issues: tuple[FailureReason, ...]


class AccessReport(ImmutableRecord):
    """A point-in-time local diagnosis; readiness never authorizes a download."""

    schema_version: Literal["loop.data-access-preflight/v1"] = Field(
        default="loop.data-access-preflight/v1", alias="schema"
    )
    checked_at: Instant
    local_ready: bool
    cache: Literal["ready", "invalid_cache"]
    sources: tuple[SourceAccess, ...] = Field(min_length=1, max_length=32)
    live_access: Literal["not_checked"] = "not_checked"
    production_eligible: Literal[False] = False


def load_access_config(path: Path) -> SourceRequest | SyncPlan:
    """Load an existing source config or sync plan, rejecting unknown TOML fields."""
    try:
        values = tomllib.loads(read_config_bytes(path).decode("utf-8"))
        content = json.dumps(values, default=_toml_date, allow_nan=False)
        if values.get("schema") == "loop.data-sync-plan/v1":
            return SyncPlan.model_validate_json(content)
        return SOURCE_CONFIG.validate_json(content)
    except OSError, ValueError, TypeError:
        raise FetchError("invalid_configuration") from None


def license_files(paths: tuple[Path, ...]) -> dict[str, LicenseFile]:
    """Read at most 32 private declarations and index their exact content hashes.

    Replacing a file later cannot change these checked bytes. Acquisition still
    rereads its path and validates the pinned digest before using the source.
    Invalid modes, special files, duplicate digests and IO errors fail closed.
    """
    if len(paths) > 32:
        raise ValueError("too many license declarations")
    result = {}
    for path in paths:
        content = read_private_config(path)
        digest = "sha256:" + hashlib.sha256(content).hexdigest()
        if digest in result:
            raise ValueError("duplicate license declaration")
        result[digest] = LicenseFile(path, content)
    return result


def _references(request: SourceRequest) -> tuple[str, ...]:
    if isinstance(request, SecRequest):
        return ()
    if isinstance(request, AlpacaRequest):
        return request.key_id_reference, request.secret_key_reference
    if isinstance(request, WrdsRequest):
        return request.username_reference, request.password_reference
    return (request.key_reference,)


def check_requests(
    requests: tuple[SourceRequest, ...],
    declarations: Mapping[str, LicenseFile],
    environment: Mapping[str, str],
    at: datetime,
    configuration: bytes,
) -> tuple[SourceAccess, ...]:
    """Check all remaining requests before IO, without retaining credential values.

    An empty tuple supports fully completed offline resume. There is no network,
    publication, file scan or caller-provided grant. Errors in any remaining
    request deny batch execution; acquisition rechecks time/rights independently.
    Sensitive input raises a static ValueError before any reference is reported.
    """
    if len(requests) > 32 or len(declarations) > 32:
        raise ValueError("access preflight exceeds its input budget")
    requests = tuple(SOURCE_CONFIG.validate_python(request) for request in requests)
    at = parse_instant(at)
    secrets = tuple(
        value
        for request in requests
        for name in (
            _references(request)[1:] if isinstance(request, WrdsRequest) else _references(request)
        )
        if (value := environment.get(name, "")) and len(value) <= 1024
    )
    # Include bounded invalid strings too: validation failure must not expose a
    # credential placed in a reference. Oversized values fail their validator;
    # they cannot equal a bounded reported reference and are never copied here.
    if secrets and any(
        contains_secret(content, secrets)
        for content in (configuration, *(item.content for item in declarations.values()))
    ):
        raise ValueError("source plan contains credential material")
    results = []
    for index, request in enumerate(requests):
        issues: list[FailureReason] = []
        references = _references(request)
        try:
            if isinstance(request, SecRequest | AlpacaRequest):
                _headers(request, environment)
            else:
                _credentials(request, environment)
        except FetchError as error:
            issues.append(error.reason)
        datasets: tuple[str, ...] = ()
        if not isinstance(request, SecRequest | AlpacaRequest):
            datasets = requested_datasets(request)
            declaration = declarations.get(request.license_sha256)
            try:
                if declaration is None:
                    raise FetchError("license_denied")
                _license(request, declaration.content, at)
            except FetchError as error:
                issues.append(error.reason)
        if request.end >= at.astimezone(ZoneInfo("America/New_York")).date() or (
            isinstance(request, WrdsRequest)
            and any(name.startswith("PG") for scope in (environment, os.environ) for name in scope)
        ):
            issues.append("invalid_configuration")
        results.append(
            SourceAccess(
                request_index=index,
                provider=request.provider,
                credential_references=references,
                missing_references=tuple(name for name in references if not environment.get(name)),
                license_datasets=datasets,
                issues=tuple(issues),
            )
        )
    return tuple(results)


def preflight(
    config: SourceRequest | SyncPlan,
    store: Path,
    *,
    licenses: tuple[Path, ...] = (),
    environment: Mapping[str, str] | None = None,
    at: datetime | None = None,
) -> AccessReport:
    """Diagnose local prerequisites only; no network, writes or secret persistence.

    Unmet access requirements are report entries. Unsafe or malformed input files
    raise ValueError/OSError. A successful report is not a capability and cannot
    prove supplier authentication, entitlement, data completeness or PIT quality.
    """
    config = (
        SyncPlan.model_validate(config)
        if isinstance(config, SyncPlan)
        else SOURCE_CONFIG.validate_python(config)
    )
    instant = parse_instant(datetime.now(UTC) if at is None else at)
    requests = config.requests if isinstance(config, SyncPlan) else (config,)
    sources = check_requests(
        requests,
        license_files(licenses),
        os.environ if environment is None else environment,
        instant,
        config.model_dump_json(by_alias=True).encode(),
    )
    cache: Literal["ready", "invalid_cache"] = "ready"
    try:
        os.close(private_directory(store))
    except OSError, ValueError:
        cache = "invalid_cache"
    return AccessReport(
        checked_at=instant,
        local_ready=cache == "ready" and all(not source.issues for source in sources),
        cache=cache,
        sources=sources,
    )
