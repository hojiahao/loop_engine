"""Bootstrap health contract for the research worker."""

from typing import Literal

from pydantic import BaseModel, ConfigDict


class ResearchHealth(BaseModel):
    """Machine-readable readiness state."""

    model_config = ConfigDict(frozen=True)

    component: Literal["researchd"] = "researchd"
    protocol_version: Literal["loop-engine.v1alpha1"] = "loop-engine.v1alpha1"
    status: Literal["ready"] = "ready"


def research_health() -> ResearchHealth:
    """Return the local bootstrap health state."""

    return ResearchHealth()
