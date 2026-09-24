from loop_research import research_health


# Scenario: research health is ready.
def test_research_health() -> None:
    health = research_health()
    assert health.component == "researchd"
    assert health.status == "ready"
    assert health.model_dump(mode="json")["protocol_version"] == "loop-engine.v1alpha1"
