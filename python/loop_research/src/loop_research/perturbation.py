"""Bounded single-window proposals; persistence and evidence belong to loopd."""

from __future__ import annotations

import math
import os
import re
import sys

import numpy as np
from google.protobuf.message import DecodeError  # type: ignore[import-untyped]
from loop.v1.perturbation_pb2 import (
    PERTURBATION_REASON_EXHAUSTED,
    PERTURBATION_REASON_EXPLORATION,
    PERTURBATION_REASON_GRADIENT,
    PerturbationState,
    PerturbationStep,
    PerturbationWork,
    WindowCandidate,
)

ALGORITHM = "window.ema-gradient-pcg64.v1"
MAX_BYTES = 1_048_576
MAX_HISTORY = 1_024
MAX_DRAWS = 1_000_000_000
_FACTOR_ID = re.compile(r"sha256:[0-9a-f]{64}\Z", re.ASCII)
_JOB_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}\Z", re.ASCII)


def _candidates(work: PerturbationWork) -> dict[str, WindowCandidate]:
    if not 2 <= len(work.candidates) <= 64:
        raise ValueError("candidate count")
    candidates: dict[str, WindowCandidate] = {}
    previous = 0
    for candidate in work.candidates:
        identity = candidate.factor_spec_id.value
        if (
            not previous < candidate.window <= 4_096
            or _FACTOR_ID.fullmatch(identity) is None
            or identity in candidates
        ):
            raise ValueError("candidate identity or window order")
        candidates[identity] = candidate
        previous = candidate.window
    if work.current_window not in {candidate.window for candidate in work.candidates}:
        raise ValueError("current window")
    return candidates


def _gradient(state: PerturbationState, center: int) -> float:
    if len(state.history) < 2:
        return 0.0
    windows = np.asarray([item.candidate.window for item in state.history], dtype=np.float64)
    sharpes = np.asarray([item.net_sharpe for item in state.history], dtype=np.float64)
    exponent = -0.5 * ((windows - center) / 5.0) ** 2
    weights = np.exp(exponent - np.max(exponent))
    weights /= weights.sum()
    dx = windows - float(np.dot(weights, windows))
    dy = sharpes - float(np.dot(weights, sharpes))
    variance = float(np.dot(weights, dx * dx))
    if variance <= np.finfo(np.float64).eps:
        return 0.0
    return float(np.dot(weights, dx * dy) / variance)


def _validate_state(state: PerturbationState, candidates: dict[str, WindowCandidate]) -> None:
    if (
        state.version != 1
        or len(state.random_seed.value) != 32
        or state.random_draws > MAX_DRAWS
        or len(state.history) > MAX_HISTORY
        or not math.isfinite(state.momentum)
        or abs(state.momentum) > 2_000_000
        or not math.isfinite(state.second_moment)
        or not 0 <= state.second_moment <= 4e12
    ):
        raise ValueError("perturbation state")
    observed: set[str] = set()
    for item in state.history:
        if (
            _JOB_ID.fullmatch(item.source_job_id.value) is None
            or item.source_job_id.value in observed
            or item.candidate != candidates.get(item.candidate.factor_spec_id.value)
            or not math.isfinite(item.net_sharpe)
            or abs(item.net_sharpe) > 1_000_000
        ):
            raise ValueError("Sharpe observation")
        observed.add(item.source_job_id.value)
    if not state.history and (state.momentum != 0 or state.second_moment != 0):
        raise ValueError("nonzero empty-state moments")
    proposed = [item.value for item in state.proposed_factor_ids]
    if len(proposed) != len(set(proposed)) or not set(proposed) <= candidates.keys():
        raise ValueError("proposed identities")


def _choose(state: PerturbationState, count: int) -> int:
    generator = np.random.PCG64(int.from_bytes(state.random_seed.value, "big"))
    generator.advance(state.random_draws)
    limit = (1 << 64) - ((1 << 64) % count)
    for _ in range(32):
        if state.random_draws >= MAX_DRAWS:
            raise ValueError("random draw budget exhausted")
        value = int(generator.random_raw())
        state.random_draws += 1
        if value < limit:
            return value % count
    raise ValueError("random rejection budget exhausted")


def advance(work: PerturbationWork) -> PerturbationStep:
    """Return a new state and an unobserved, non-failed candidate without I/O.

    Cold start explores instead of silently returning the original window.
    A source job is observed once. Replay of an identical observation is harmless;
    conflicting reuse is an error. The caller owns authority and atomic commit.
    """
    candidates = _candidates(work)
    state = PerturbationState()
    state.CopyFrom(work.state)
    _validate_state(state, candidates)
    failed = [identity.value for identity in work.failed_factor_ids]
    if len(failed) > 64 or len(set(failed)) != len(failed) or not set(failed) <= candidates.keys():
        raise ValueError("failed candidate identities")
    if work.HasField("observation"):
        observation = work.observation
        previous = next(
            (item for item in state.history if item.source_job_id == observation.source_job_id),
            None,
        )
        if previous is not None and previous != observation:
            raise ValueError("source job observation conflict")
        if previous is None:
            if len(state.history) == MAX_HISTORY:
                raise ValueError("history budget exhausted")
            state.history.append(observation)
            _validate_state(state, candidates)
            gradient = _gradient(state, observation.candidate.window)
            state.momentum = 0.7 * state.momentum + 0.3 * gradient
            state.second_moment = 0.7 * state.second_moment + 0.3 * gradient * gradient
    _validate_state(state, candidates)
    observed = {item.candidate.factor_spec_id.value for item in state.history}
    excluded = observed | set(failed) | {item.value for item in state.proposed_factor_ids}
    available = [item for identity, item in candidates.items() if identity not in excluded]
    if not available:
        return PerturbationStep(state=state, reason=PERTURBATION_REASON_EXHAUSTED)
    reason = PERTURBATION_REASON_EXPLORATION
    if len({item.candidate.window for item in state.history}) >= 2 and state.momentum != 0:
        target = work.current_window + 2.0 * state.momentum / (
            math.sqrt(state.second_moment) + 1e-8
        )
        distance = min(abs(item.window - target) for item in available)
        available = [item for item in available if abs(item.window - target) == distance]
        reason = PERTURBATION_REASON_GRADIENT
    chosen = available[_choose(state, len(available))]
    state.proposed_factor_ids.append(chosen.factor_spec_id)
    return PerturbationStep(state=state, candidate=chosen, reason=reason)


def main() -> int:
    """Consume/produce one bounded Protobuf envelope; never read data or secrets."""
    try:
        payload = sys.stdin.buffer.read(MAX_BYTES + 1)
        if not payload or len(payload) > MAX_BYTES:
            raise ValueError("worker input size")
        work = PerturbationWork.FromString(payload)
        source = os.environ.get("LOOP_ENGINE_BUILD_SOURCE_SHA256")
        environment = os.environ.get("LOOP_ENGINE_BUILD_ENVIRONMENT_SHA256")
        if source is not None or environment is not None:
            if source is None or environment is None:
                raise ValueError("incomplete worker build identity")
            from loop_research.build_identity import require_build

            require_build(source, environment)
        output = advance(work).SerializeToString()
        if source is not None and environment is not None:
            require_build(source, environment)
        if len(output) > MAX_BYTES:
            raise ValueError("worker output size")
    except ValueError, DecodeError, OverflowError, OSError:
        sys.stderr.write("invalid perturbation work\n")
        return 2
    sys.stdout.buffer.write(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
