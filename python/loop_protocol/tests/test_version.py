from loop_protocol import PROTOCOL_VERSION


# Scenario: protocol version is namespaced.
def test_protocol_version() -> None:
    assert PROTOCOL_VERSION.startswith("loop-engine.")
