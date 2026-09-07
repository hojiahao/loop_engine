"""Pure protocol availability and negotiation checks for preflight gates."""

from __future__ import annotations

import hmac
import re
from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum
from itertools import pairwise

from loop.v1 import common_pb2

from .job import JobValidationError, protocol_selection_sha256

_MAX_PACKAGES = 16
_MAX_FEATURES = 256
_MAX_NAME_BYTES = 128
_MAX_BUILD_VERSION_BYTES = 128
_MAX_UNARY_BYTES = 4_194_304
_MAX_STREAM_EVENT_BYTES = 1_048_576
_MAX_CANONICAL_AST_BYTES = 262_144
_MAX_AST_NODES = 4_096
_MAX_AST_DEPTH = 64
_MAX_PAGE_RECORDS = 500
_MAX_IDENTITY_BYTES = 128
_MAX_ARTIFACT_URI_BYTES = 2_048
_PROTOCOL_PACKAGE_RE = re.compile(
    r"^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*\.v[1-9][0-9]*$", re.ASCII
)
_PROTOCOL_FEATURE_RE = re.compile(
    r"^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$",
    re.ASCII,
)
_BUILD_VERSION_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9.+_-]*$", re.ASCII)


class ProtocolNegotiationCode(StrEnum):
    """Stable fail-closed preflight result codes shared by all runtimes."""

    INVALID_PROTOCOL_INFO = "invalid_protocol_info"
    INVALID_REQUIREMENT = "invalid_requirement"
    UNSUPPORTED_MAJOR = "unsupported_major"
    UNSUPPORTED_REQUIRED_FEATURE = "unsupported_required_feature"
    UNSUPPORTED_FEATURE = "unsupported_feature"
    INCOMPATIBLE_LIMITS = "incompatible_limits"
    INVALID_SELECTION = "invalid_selection"
    UNAVAILABLE_BUILD = "unavailable_build"
    UNAVAILABLE_DESCRIPTOR = "unavailable_descriptor"


class ProtocolNegotiationError(ValueError):
    """Protocol availability cannot satisfy a preflight requirement."""

    def __init__(self, code: ProtocolNegotiationCode) -> None:
        self.code = code
        super().__init__(code.value)


@dataclass(frozen=True, slots=True)
class ProtocolBuildIdentity:
    build_version: str
    build_sha256: bytes


@dataclass(frozen=True, slots=True)
class NegotiatedProtocol:
    selected_package: str
    enabled_features: tuple[str, ...]
    effective_limits: common_pb2.ProtocolLimits
    local_build: ProtocolBuildIdentity
    peer_build: ProtocolBuildIdentity


def validate_protocol_info(info: common_pb2.ProtocolInfo) -> None:
    """Validate the closed availability projection advertised by one peer."""

    valid = (
        1 <= len(info.supported_packages) <= _MAX_PACKAGES
        and _is_sorted_unique(info.supported_packages)
        and all(_is_protocol_package(value) for value in info.supported_packages)
        and len(info.features) <= _MAX_FEATURES
        and _is_sorted_unique(info.features)
        and all(_is_protocol_feature(value) for value in info.features)
        and info.HasField("limits")
        and _is_valid_limits(info.limits)
        and _is_build_version(info.build_version)
        and info.HasField("build_sha256")
        and len(info.build_sha256.value) == 32
    )
    if not valid:
        raise ProtocolNegotiationError(ProtocolNegotiationCode.INVALID_PROTOCOL_INFO)


def negotiate_protocol_availability(
    local: common_pb2.ProtocolInfo,
    peer: common_pb2.ProtocolInfo,
    required_package: str,
    required_features: Sequence[str],
) -> NegotiatedProtocol:
    """Negotiate one required service major before mutable or paid work."""

    validate_protocol_info(local)
    validate_protocol_info(peer)
    _validate_requirement(required_package, required_features)
    if (
        required_package not in local.supported_packages
        or required_package not in peer.supported_packages
    ):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.UNSUPPORTED_MAJOR)

    peer_features = frozenset(peer.features)
    enabled_features = tuple(feature for feature in local.features if feature in peer_features)
    if any(feature not in enabled_features for feature in required_features):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.UNSUPPORTED_REQUIRED_FEATURE)

    return NegotiatedProtocol(
        selected_package=required_package,
        enabled_features=enabled_features,
        effective_limits=_minimum_limits(local.limits, peer.limits),
        local_build=_protocol_info_build(local),
        peer_build=_protocol_info_build(peer),
    )


def validate_protocol_selection_availability(
    selection: common_pb2.ProtocolSelectionSnapshot,
    local: common_pb2.ProtocolInfo,
    retained_builds: Sequence[ProtocolBuildIdentity],
    available_schema_descriptors: Sequence[bytes],
    required_package: str,
    required_features: Sequence[str],
) -> None:
    """Prove a pinned selection remains executable from local immutable content."""

    validate_protocol_info(local)
    _validate_requirement(required_package, required_features)
    try:
        computed = protocol_selection_sha256(selection)
    except JobValidationError as error:
        raise ProtocolNegotiationError(ProtocolNegotiationCode.INVALID_SELECTION) from error
    if (
        not selection.HasField("selection_sha256")
        or len(selection.selection_sha256.value) != 32
        or not hmac.compare_digest(selection.selection_sha256.value, computed)
    ):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.INVALID_SELECTION)
    if (
        selection.selected_package != required_package
        or selection.selected_package not in local.supported_packages
    ):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.UNSUPPORTED_MAJOR)
    if any(feature not in selection.enabled_features for feature in required_features):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.UNSUPPORTED_REQUIRED_FEATURE)
    if any(feature not in local.features for feature in selection.enabled_features):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.UNSUPPORTED_FEATURE)
    if not _limits_fit(selection.effective_limits, local.limits):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.INCOMPATIBLE_LIMITS)

    server_build = _selection_build(
        selection.server_build_version,
        selection.server_build_sha256.value,
    )
    client_build = _selection_build(
        selection.client_build_version,
        selection.client_build_sha256.value,
    )
    if not _build_is_available(server_build, local, retained_builds) or not _build_is_available(
        client_build, local, retained_builds
    ):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.UNAVAILABLE_BUILD)
    descriptor = selection.schema_descriptor_sha256.value
    if not any(
        hmac.compare_digest(descriptor, available) for available in available_schema_descriptors
    ):
        raise ProtocolNegotiationError(ProtocolNegotiationCode.UNAVAILABLE_DESCRIPTOR)


def _validate_requirement(required_package: str, required_features: Sequence[str]) -> None:
    valid = (
        _is_protocol_package(required_package)
        and len(required_features) <= _MAX_FEATURES
        and _is_sorted_unique(required_features)
        and all(_is_protocol_feature(value) for value in required_features)
    )
    if not valid:
        raise ProtocolNegotiationError(ProtocolNegotiationCode.INVALID_REQUIREMENT)


def _protocol_info_build(info: common_pb2.ProtocolInfo) -> ProtocolBuildIdentity:
    return ProtocolBuildIdentity(info.build_version, bytes(info.build_sha256.value))


def _selection_build(version: str, digest: bytes) -> ProtocolBuildIdentity:
    if not _is_build_version(version) or len(digest) != 32:
        raise ProtocolNegotiationError(ProtocolNegotiationCode.INVALID_SELECTION)
    return ProtocolBuildIdentity(version, bytes(digest))


def _build_is_available(
    requested: ProtocolBuildIdentity,
    local: common_pb2.ProtocolInfo,
    retained: Sequence[ProtocolBuildIdentity],
) -> bool:
    return _build_identity_equal(requested, _protocol_info_build(local)) or any(
        _build_identity_equal(requested, available) for available in retained
    )


def _build_identity_equal(left: ProtocolBuildIdentity, right: ProtocolBuildIdentity) -> bool:
    return left.build_version == right.build_version and hmac.compare_digest(
        left.build_sha256, right.build_sha256
    )


def _minimum_limits(
    left: common_pb2.ProtocolLimits, right: common_pb2.ProtocolLimits
) -> common_pb2.ProtocolLimits:
    return common_pb2.ProtocolLimits(
        maximum_unary_bytes=min(left.maximum_unary_bytes, right.maximum_unary_bytes),
        maximum_stream_event_bytes=min(
            left.maximum_stream_event_bytes, right.maximum_stream_event_bytes
        ),
        maximum_canonical_ast_bytes=min(
            left.maximum_canonical_ast_bytes, right.maximum_canonical_ast_bytes
        ),
        maximum_ast_nodes=min(left.maximum_ast_nodes, right.maximum_ast_nodes),
        maximum_ast_depth=min(left.maximum_ast_depth, right.maximum_ast_depth),
        maximum_page_records=min(left.maximum_page_records, right.maximum_page_records),
        maximum_identity_bytes=min(left.maximum_identity_bytes, right.maximum_identity_bytes),
        maximum_artifact_uri_bytes=min(
            left.maximum_artifact_uri_bytes, right.maximum_artifact_uri_bytes
        ),
    )


def _limits_fit(selected: common_pb2.ProtocolLimits, local: common_pb2.ProtocolLimits) -> bool:
    return (
        selected.maximum_unary_bytes <= local.maximum_unary_bytes
        and selected.maximum_stream_event_bytes <= local.maximum_stream_event_bytes
        and selected.maximum_canonical_ast_bytes <= local.maximum_canonical_ast_bytes
        and selected.maximum_ast_nodes <= local.maximum_ast_nodes
        and selected.maximum_ast_depth <= local.maximum_ast_depth
        and selected.maximum_page_records <= local.maximum_page_records
        and selected.maximum_identity_bytes <= local.maximum_identity_bytes
        and selected.maximum_artifact_uri_bytes <= local.maximum_artifact_uri_bytes
    )


def _is_valid_limits(limits: common_pb2.ProtocolLimits) -> bool:
    return (
        1 <= limits.maximum_unary_bytes <= _MAX_UNARY_BYTES
        and 1 <= limits.maximum_stream_event_bytes <= _MAX_STREAM_EVENT_BYTES
        and 1 <= limits.maximum_canonical_ast_bytes <= _MAX_CANONICAL_AST_BYTES
        and 1 <= limits.maximum_ast_nodes <= _MAX_AST_NODES
        and 1 <= limits.maximum_ast_depth <= _MAX_AST_DEPTH
        and 1 <= limits.maximum_page_records <= _MAX_PAGE_RECORDS
        and 1 <= limits.maximum_identity_bytes <= _MAX_IDENTITY_BYTES
        and 1 <= limits.maximum_artifact_uri_bytes <= _MAX_ARTIFACT_URI_BYTES
        and limits.maximum_canonical_ast_bytes <= limits.maximum_unary_bytes
        and limits.maximum_identity_bytes <= limits.maximum_unary_bytes
        and limits.maximum_artifact_uri_bytes <= limits.maximum_unary_bytes
    )


def _is_sorted_unique(values: Sequence[str]) -> bool:
    return all(left < right for left, right in pairwise(values))


def _is_protocol_package(value: str) -> bool:
    return len(value.encode("utf-8")) <= _MAX_NAME_BYTES and bool(
        _PROTOCOL_PACKAGE_RE.fullmatch(value)
    )


def _is_protocol_feature(value: str) -> bool:
    return len(value.encode("utf-8")) <= _MAX_NAME_BYTES and bool(
        _PROTOCOL_FEATURE_RE.fullmatch(value)
    )


def _is_build_version(value: str) -> bool:
    return len(value.encode("utf-8")) <= _MAX_BUILD_VERSION_BYTES and bool(
        _BUILD_VERSION_RE.fullmatch(value)
    )
