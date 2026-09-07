from __future__ import annotations

import csv
from pathlib import Path

import pytest

from loop.v1 import common_pb2
from loop_protocol import (
    ProtocolBuildIdentity,
    ProtocolNegotiationError,
    negotiate_protocol_availability,
    protocol_selection_sha256,
    validate_protocol_selection_availability,
)

VECTORS = Path(__file__).parents[3] / "tests" / "contracts" / "protocol_negotiation_vectors.tsv"
SHARED_VECTORS = list(csv.DictReader(VECTORS.open(), delimiter="\t"))
REQUIRED_PACKAGE = "loop.research.v1"
REQUIRED_FEATURES = ("jobs.envelope.v1",)


@pytest.mark.parametrize("vector", SHARED_VECTORS, ids=lambda vector: vector["name"])
def test_shared_protocol_negotiation_matrix_fails_closed(vector: dict[str, str]) -> None:
    assert len(SHARED_VECTORS) == 15
    operation = vector["operation"]
    if operation == "negotiate":
        result = _run_negotiation(vector["mutation"])
    elif operation == "selection":
        result = _run_selection_validation(vector["mutation"])
    else:
        raise AssertionError(f"unknown operation {operation}")
    assert result == vector["expected"]


def _run_negotiation(mutation: str) -> str:
    local = _protocol_info("client.1", 0x11, local=True)
    peer = _protocol_info("server.1", 0x22, local=False)
    if mutation == "none":
        pass
    elif mutation == "local_remove_package":
        del local.supported_packages[0]
    elif mutation == "peer_remove_package":
        del peer.supported_packages[0]
    elif mutation == "local_remove_required_feature":
        local.features.remove(REQUIRED_FEATURES[0])
    elif mutation == "peer_remove_required_feature":
        peer.features.remove(REQUIRED_FEATURES[0])
    elif mutation == "peer_unsorted_packages":
        peer.supported_packages.reverse()
    else:
        raise AssertionError(f"unknown negotiation mutation {mutation}")

    try:
        negotiated = negotiate_protocol_availability(
            local, peer, REQUIRED_PACKAGE, REQUIRED_FEATURES
        )
    except ProtocolNegotiationError as error:
        return error.code.value
    assert negotiated.enabled_features == (
        "factors.canonical-json.v1",
        "jobs.envelope.v1",
    )
    assert negotiated.effective_limits == peer.limits
    return "accept"


def _run_selection_validation(mutation: str) -> str:
    local = _protocol_info("client.1", 0x11, local=True)
    selection = _protocol_selection()
    recompute_digest = True
    if mutation == "none":
        pass
    elif mutation == "selection_package":
        selection.selected_package = "loop.research.v2"
    elif mutation == "selection_remove_required_feature":
        del selection.enabled_features[1]
    elif mutation == "selection_add_unsupported_feature":
        selection.enabled_features.append("streams.sequence.v1")
    elif mutation == "selection_limit_exceeds_local":
        selection.effective_limits.maximum_unary_bytes = 3_145_728
    elif mutation == "selection_server_build":
        selection.server_build_version = "server.2"
    elif mutation == "selection_client_build":
        selection.client_build_sha256.CopyFrom(_digest(0x44))
    elif mutation == "selection_descriptor":
        selection.schema_descriptor_sha256.CopyFrom(_digest(0x55))
    elif mutation == "selection_digest":
        claimed = bytearray(selection.selection_sha256.value)
        claimed[0] ^= 0xFF
        selection.selection_sha256.value = bytes(claimed)
        recompute_digest = False
    else:
        raise AssertionError(f"unknown selection mutation {mutation}")
    if recompute_digest:
        selection.selection_sha256.value = protocol_selection_sha256(selection)

    retained_builds = (ProtocolBuildIdentity("server.1", bytes([0x22]) * 32),)
    try:
        validate_protocol_selection_availability(
            selection,
            local,
            retained_builds,
            (bytes([0x33]) * 32,),
            REQUIRED_PACKAGE,
            REQUIRED_FEATURES,
        )
    except ProtocolNegotiationError as error:
        return error.code.value
    return "accept"


def _protocol_info(build_version: str, build_byte: int, *, local: bool) -> common_pb2.ProtocolInfo:
    features = (
        (
            "artifacts.by-reference.v1",
            "factors.canonical-json.v1",
            "jobs.envelope.v1",
        )
        if local
        else (
            "factors.canonical-json.v1",
            "jobs.envelope.v1",
            "streams.sequence.v1",
        )
    )
    limits = (
        _limits(2_097_152, 524_288, 131_072, 2_048, 32, 250, 128, 1_024)
        if local
        else _limits(1_048_576, 262_144, 65_536, 1_024, 16, 100, 64, 512)
    )
    return common_pb2.ProtocolInfo(
        supported_packages=("loop.research.v1", "loop.v1"),
        features=features,
        limits=limits,
        build_version=build_version,
        build_sha256=_digest(build_byte),
    )


def _protocol_selection() -> common_pb2.ProtocolSelectionSnapshot:
    selection = common_pb2.ProtocolSelectionSnapshot(
        selected_package=REQUIRED_PACKAGE,
        enabled_features=("factors.canonical-json.v1", "jobs.envelope.v1"),
        effective_limits=_limits(1_048_576, 262_144, 65_536, 1_024, 16, 100, 64, 512),
        server_build_version="server.1",
        server_build_sha256=_digest(0x22),
        schema_descriptor_sha256=_digest(0x33),
        client_build_version="client.1",
        client_build_sha256=_digest(0x11),
    )
    selection.selected_at.seconds = 2
    selection.selection_sha256.value = protocol_selection_sha256(selection)
    return selection


def _limits(
    unary: int,
    stream: int,
    ast: int,
    nodes: int,
    depth: int,
    page: int,
    identity: int,
    uri: int,
) -> common_pb2.ProtocolLimits:
    return common_pb2.ProtocolLimits(
        maximum_unary_bytes=unary,
        maximum_stream_event_bytes=stream,
        maximum_canonical_ast_bytes=ast,
        maximum_ast_nodes=nodes,
        maximum_ast_depth=depth,
        maximum_page_records=page,
        maximum_identity_bytes=identity,
        maximum_artifact_uri_bytes=uri,
    )


def _digest(byte: int) -> common_pb2.Sha256Digest:
    return common_pb2.Sha256Digest(value=bytes([byte]) * 32)
