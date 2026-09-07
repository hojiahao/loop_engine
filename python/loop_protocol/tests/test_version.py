from loop_protocol import PROTOCOL_VERSION


def test_protocol_version_is_namespaced() -> None:
    assert PROTOCOL_VERSION.startswith("loop-engine.")
