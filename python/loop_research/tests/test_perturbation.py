import math
import subprocess
import sys

import pytest
from hypothesis import given
from hypothesis import strategies as st
from loop.v1.common_pb2 import FactorSpecId, JobId, Sha256Digest
from loop.v1.perturbation_pb2 import (
    PERTURBATION_REASON_EXHAUSTED,
    PERTURBATION_REASON_EXPLORATION,
    PERTURBATION_REASON_GRADIENT,
    PerturbationState,
    PerturbationStep,
    PerturbationWork,
    WindowCandidate,
    WindowObservation,
)

from loop_research.perturbation import MAX_BYTES, MAX_DRAWS, advance


def work() -> PerturbationWork:
    candidates = [
        WindowCandidate(window=window, factor_spec_id=FactorSpecId(value=f"sha256:{window:064x}"))
        for window in (5, 10, 15, 20, 25)
    ]
    return PerturbationWork(
        state=PerturbationState(version=1, random_seed=Sha256Digest(value=bytes(32))),
        candidates=candidates,
        current_window=15,
        observation=WindowObservation(
            source_job_id=JobId(value="job.1"), candidate=candidates[2], net_sharpe=1.0
        ),
    )


def run_worker(payload: bytes) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        [sys.executable, "-I", "-m", "loop_research.perturbation"],
        input=payload,
        capture_output=True,
        timeout=10,
        check=False,
        env={},
    )


def test_cold_start_proposes_new_window() -> None:
    request = work()
    before = request.SerializeToString()
    step = advance(request)
    assert step.reason == PERTURBATION_REASON_EXPLORATION
    assert step.candidate.window == 25
    assert step.state.random_draws == 1
    assert list(step.state.history) == [request.observation]
    assert request.SerializeToString() == before


def test_gradient_uses_sharpe_history() -> None:
    request = work()
    request.state.history.append(
        WindowObservation(
            source_job_id=JobId(value="job.previous"),
            candidate=request.candidates[1],
            net_sharpe=1,
        )
    )
    request.observation.candidate.CopyFrom(request.candidates[3])
    request.observation.net_sharpe = 3
    request.current_window = 20
    result = advance(request)
    assert result.reason == PERTURBATION_REASON_GRADIENT
    assert result.state.momentum == pytest.approx(0.06)
    assert result.state.second_moment == pytest.approx(0.012)
    assert result.candidate.window == 25


def test_restart_matches_continuous_sequence() -> None:
    request = work()
    for _ in range(5):
        expected = advance(request)
        process = run_worker(request.SerializeToString())
        assert process.returncode == 0, process.stderr
        assert PerturbationStep.FromString(process.stdout) == expected
        request.state.CopyFrom(expected.state)
    assert expected.reason == PERTURBATION_REASON_EXHAUSTED
    assert len(expected.state.history) == 1
    assert len(expected.state.proposed_factor_ids) == 4
    assert expected.state.random_draws == 4


def test_failed_candidates_are_excluded() -> None:
    request = work()
    request.failed_factor_ids.extend(
        candidate.factor_spec_id for candidate in request.candidates[1:]
    )
    assert advance(request).candidate == request.candidates[0]


def test_exhaustion_does_not_consume_randomness() -> None:
    request = work()
    request.failed_factor_ids.extend(candidate.factor_spec_id for candidate in request.candidates)
    result = advance(request)
    assert not result.HasField("candidate")
    assert result.reason == PERTURBATION_REASON_EXHAUSTED
    assert result.state.random_draws == 0


def test_same_source_is_observed_once() -> None:
    request = work()
    first = advance(request)
    request.state.CopyFrom(first.state)
    second = advance(request)
    assert len(second.state.history) == 1
    assert second.candidate != first.candidate
    assert second.state.momentum == first.state.momentum


def test_conflicting_source_is_rejected() -> None:
    request = work()
    request.state.CopyFrom(advance(request).state)
    request.observation.net_sharpe = 2
    with pytest.raises(ValueError, match="source job observation conflict"):
        advance(request)


@pytest.mark.parametrize("value", [math.nan, math.inf, -math.inf, 1_000_001.0])
def test_invalid_sharpe_is_rejected(value: float) -> None:
    request = work()
    request.observation.net_sharpe = value
    with pytest.raises(ValueError):
        advance(request)


@pytest.mark.parametrize("field", ["momentum", "second_moment"])
@pytest.mark.parametrize("value", [math.nan, math.inf, -math.inf])
def test_nonfinite_state_is_rejected(field: str, value: float) -> None:
    request = work()
    setattr(request.state, field, value)
    with pytest.raises(ValueError):
        advance(request)


@pytest.mark.parametrize("invalid", ["version", "seed", "order", "duplicate", "unknown", "draws"])
def test_invalid_work_is_rejected(invalid: str) -> None:
    request = work()
    if invalid == "version":
        request.state.version = 2
    elif invalid == "seed":
        request.state.random_seed.value = b"short"
    elif invalid == "order":
        request.candidates.reverse()
    elif invalid == "duplicate":
        request.candidates[0].factor_spec_id.CopyFrom(request.candidates[1].factor_spec_id)
    elif invalid == "unknown":
        request.failed_factor_ids.append(FactorSpecId(value=f"sha256:{999:064x}"))
    else:
        request.state.random_draws = MAX_DRAWS
    with pytest.raises(ValueError):
        advance(request)


def test_history_budget_is_not_silently_truncated() -> None:
    request = work()
    request.state.history.extend(
        WindowObservation(
            source_job_id=JobId(value=f"old.{index}"),
            candidate=request.candidates[0],
            net_sharpe=0,
        )
        for index in range(1024)
    )
    with pytest.raises(ValueError, match="history budget"):
        advance(request)


@pytest.mark.parametrize("payload", [b"", b"\xff", b"x" * (MAX_BYTES + 1)])
def test_worker_rejects_invalid_envelopes(payload: bytes) -> None:
    result = run_worker(payload)
    assert result.returncode == 2
    assert result.stdout == b""
    assert result.stderr == b"invalid perturbation work\n"


@given(st.binary(min_size=32, max_size=32), st.sets(st.integers(0, 4), max_size=5))
def test_proposals_are_bounded_and_never_repeat(seed: bytes, failures: set[int]) -> None:
    request = work()
    request.state.random_seed.value = seed
    request.failed_factor_ids.extend(
        request.candidates[index].factor_spec_id for index in sorted(failures)
    )
    seen = {request.observation.candidate.factor_spec_id.value}
    failed = {item.value for item in request.failed_factor_ids}
    for _ in range(5):
        result = advance(request)
        if not result.HasField("candidate"):
            assert len(seen | failed) == 5
            break
        identity = result.candidate.factor_spec_id.value
        assert identity not in seen | failed
        seen.add(identity)
        request.state.CopyFrom(result.state)
