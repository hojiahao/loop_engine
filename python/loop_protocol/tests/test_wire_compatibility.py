import hashlib
import json
from pathlib import Path
from typing import Any

from loop.protocol.v1 import service_pb2
from loop.v1 import common_pb2

FIXTURE_DIRECTORY = (
    Path(__file__).resolve().parents[3] / "fixtures" / "contracts" / "protocol" / "v1"
)
SUPPORTED_PACKAGES = [
    "loop.audit.v1",
    "loop.discovery.v1",
    "loop.holdout.v1",
    "loop.jobs.v1",
    "loop.protocol.v1",
    "loop.provider.v1",
    "loop.research.v1",
    "loop.v1",
]
FEATURES = [
    "artifacts.by-reference.v1",
    "factors.canonical-json.v1",
]
PRODUCER_FIXTURES = [
    "protocol_info_v1.binpb",
    "protocol_info_v1.rust.binpb",
    "protocol_info_v1.typescript.binpb",
]


def _sha256(data: bytes) -> str:
    return f"sha256:{hashlib.sha256(data).hexdigest()}"


def _fixture(name: str) -> bytes:
    return (FIXTURE_DIRECTORY / name).read_bytes()


def _assert_expected_projection(message: common_pb2.ProtocolInfo) -> None:
    assert list(message.supported_packages) == SUPPORTED_PACKAGES
    assert list(message.features) == FEATURES
    assert message.limits == common_pb2.ProtocolLimits(
        maximum_unary_bytes=4_194_304,
        maximum_stream_event_bytes=1_048_576,
        maximum_canonical_ast_bytes=262_144,
        maximum_ast_nodes=4_096,
        maximum_ast_depth=64,
        maximum_page_records=500,
        maximum_identity_bytes=128,
        maximum_artifact_uri_bytes=2_048,
    )
    assert message.build_version == "0.2.0-alpha.1+wire-fixture.1"
    assert message.build_sha256.value == bytes(range(32))


def _expected_protocol_info() -> common_pb2.ProtocolInfo:
    return common_pb2.ProtocolInfo(
        supported_packages=SUPPORTED_PACKAGES,
        features=FEATURES,
        limits=common_pb2.ProtocolLimits(
            maximum_unary_bytes=4_194_304,
            maximum_stream_event_bytes=1_048_576,
            maximum_canonical_ast_bytes=262_144,
            maximum_ast_nodes=4_096,
            maximum_ast_depth=64,
            maximum_page_records=500,
            maximum_identity_bytes=128,
            maximum_artifact_uri_bytes=2_048,
        ),
        build_version="0.2.0-alpha.1+wire-fixture.1",
        build_sha256=common_pb2.Sha256Digest(value=bytes(range(32))),
    )


def _assert_semantic_round_trip(name: str) -> None:
    decoded = common_pb2.ProtocolInfo.FromString(_fixture(name))
    _assert_expected_projection(decoded)

    locally_encoded = decoded.SerializeToString(deterministic=True)
    decoded_again = common_pb2.ProtocolInfo.FromString(locally_encoded)
    _assert_expected_projection(decoded_again)


def test_generated_bindings_import() -> None:
    assert common_pb2.ProtocolInfo.DESCRIPTOR.full_name == "loop.v1.ProtocolInfo"
    assert (
        service_pb2.GetProtocolInfoRequest.DESCRIPTOR.full_name
        == "loop.protocol.v1.GetProtocolInfoRequest"
    )


def test_wire_fixture_manifest_matches_committed_bytes_and_schema() -> None:
    manifest: dict[str, Any] = json.loads(
        (FIXTURE_DIRECTORY / "wire_fixtures.json").read_text(encoding="ascii")
    )
    assert manifest["fixture_schema"] == "loop.contract-fixture-manifest/v2"
    assert manifest["message_type"] == "loop.v1.ProtocolInfo"
    assert manifest["schema_descriptor"]["sha256"] == _sha256(
        (FIXTURE_DIRECTORY / "schema.current.binpb").read_bytes()
    )
    assert {record["language"] for record in manifest["producers"]} == {
        "python",
        "rust",
        "typescript",
    }
    for record in manifest["producers"]:
        assert record["wire_sha256"] == _sha256(_fixture(record["path"]))
        assert record["toolchain"]
        source = record["source"]
        repository = FIXTURE_DIRECTORY.parents[3]
        assert source["sha256"] == _sha256((repository / source["path"]).read_bytes())
    for record in manifest["compatibility_fixtures"]:
        assert record["wire_sha256"] == _sha256(_fixture(record["path"]))
        assert record["lossless_original_byte_forwarding_required"] is False
    for record in manifest["semantic_negative_fixtures"]:
        assert record["wire_sha256"] == _sha256(_fixture(record["path"]))
    operational = manifest["auxiliary_contracts"][0]
    assert operational["sha256"] == _sha256(
        (FIXTURE_DIRECTORY.parents[3] / operational["path"]).read_bytes()
    )


def test_python_fixture_is_current_native_encoder_output() -> None:
    assert _fixture("protocol_info_v1.binpb") == _expected_protocol_info().SerializeToString(
        deterministic=True
    )


def test_decodes_and_reencodes_every_producer_fixture() -> None:
    for name in PRODUCER_FIXTURES:
        _assert_semantic_round_trip(name)


def test_old_reader_tolerates_additive_unknown_field() -> None:
    _assert_semantic_round_trip("protocol_info_v1_unknown_field.binpb")
    # Unknown-field preservation is intentionally not asserted. Lossless
    # forwarding retains the original envelope instead of parse/re-serialize.
