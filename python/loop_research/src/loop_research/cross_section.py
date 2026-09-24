"""Equal-weight, session-local transformations with explicit degeneracy outcomes."""

import math
import re
from dataclasses import dataclass, replace
from datetime import date
from typing import Literal

import numpy as np
from numpy.typing import NDArray

from loop_research.evaluator import MAX_CELLS, MAX_WORK, Evaluation, Panel, _frozen
from loop_research.numerics import _centered_unit
from loop_research.transform_models import TransformPolicy

RCOND = 1e-12
MAX_DESIGN_COLUMNS = 64
INDUSTRY = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,63}\Z", re.ASCII)
type Outcome = Literal["ok", "insufficient", "rank_deficient", "constant"]


@dataclass(frozen=True, slots=True)
class Exposures:
    """Immutable aligned inputs; the artifact loader must prove time visibility."""

    sessions: tuple[date, ...]
    securities: tuple[str, ...]
    industry: tuple[tuple[str | None, ...], ...]
    market_cap: NDArray[np.float64]
    beta: NDArray[np.float64]

    def __post_init__(self) -> None:
        shape = len(self.sessions), len(self.securities)
        if (
            not all(shape)
            or shape[0] * shape[1] > MAX_CELLS
            or not isinstance(self.industry, tuple)
            or len(self.industry) != shape[0]
            or any(
                not isinstance(row, tuple)
                or len(row) != shape[1]
                or any(
                    value is not None and (type(value) is not str or not INDUSTRY.fullmatch(value))
                    for value in row
                )
                for row in self.industry
            )
        ):
            raise ValueError("exposure grid or industry bounds")
        for name in ("market_cap", "beta"):
            values = getattr(self, name)
            if (
                not isinstance(values, np.ndarray)
                or isinstance(values, np.ma.MaskedArray)
                or values.dtype != np.float64
                or values.shape != shape
                or np.isinf(values).any()
                or (name == "market_cap" and np.any(values <= 0))
            ):
                raise ValueError("exposures require finite-or-missing aligned numeric values")
            object.__setattr__(self, name, _frozen(values))


@dataclass(frozen=True, slots=True)
class Transformed:
    evaluation: Evaluation
    raw_valid_observations: int
    outcomes: tuple[Outcome, ...]


def _quantile(ordered: NDArray[np.float64], bps: int) -> float:
    lower, remainder = divmod((ordered.size - 1) * bps, 10000)
    left = float(ordered[lower])
    if remainder == 0:
        return left
    right = float(ordered[lower + 1])
    weight = remainder / 10000
    difference = right - left
    if math.isfinite(difference):
        return left + difference * weight
    scale = max(abs(left), abs(right))
    return math.fsum((left / scale * (1 - weight), right / scale * weight)) * scale


def _clip(values: NDArray[np.float64], bps: int) -> NDArray[np.float64]:
    if bps == 0:
        return values.copy()
    ordered = np.sort(values, kind="stable")
    return np.clip(values, _quantile(ordered, bps), _quantile(ordered, 10000 - bps))


def _continuous(values: NDArray[np.float64]) -> NDArray[np.float64]:
    centered = _centered_unit(values)
    deviation = math.sqrt(float(np.mean(centered * centered)))
    return centered if deviation == 0 else centered / deviation


def _design(
    exposures: Exposures, policy: TransformPolicy, index: int, valid: NDArray[np.bool_]
) -> NDArray[np.float64]:
    groups = tuple(
        value for value, keep in zip(exposures.industry[index], valid, strict=True) if keep
    )
    levels = sorted({value for value in groups if value is not None}) if policy.industry else []
    columns = 1 + max(0, len(levels) - 1) + int(policy.log_size) + int(policy.beta)
    if columns > MAX_DESIGN_COLUMNS:
        raise ValueError("neutralization design column budget")
    design = [np.ones(int(np.count_nonzero(valid)), dtype=np.float64)]
    design.extend(
        np.asarray([value == level for value in groups], dtype=np.float64) for level in levels[1:]
    )
    if policy.log_size:
        design.append(_continuous(np.log(exposures.market_cap[index, valid])))
    if policy.beta:
        design.append(_continuous(exposures.beta[index, valid]))
    return np.column_stack(design)


def transform(
    result: Evaluation, panel: Panel, policy: TransformPolicy, exposures: Exposures | None
) -> Transformed:
    """Clip, neutralize and optionally standardize each evaluation session.

    Keep the eligible denominator when exposures are missing. Insufficient or
    rank-deficient sessions stay missing; unexpected numerical failures raise.
    No input mutation, direction change, cross-date fit or output publication.
    """
    if policy.needs_exposures != (exposures is not None):
        raise ValueError("declared transformation exposure set differs")
    if exposures is not None and (
        exposures.sessions != panel.sessions or exposures.securities != panel.securities
    ):
        raise ValueError("transformation exposure axes differ")
    start = panel.sessions.index(result.evaluation_start)
    expected = len(panel.sessions) - start, len(panel.securities)
    if (
        not isinstance(result.values, np.ndarray)
        or isinstance(result.values, np.ma.MaskedArray)
        or result.values.dtype != np.float64
        or result.values.shape != expected
        or np.isinf(result.values).any()
        or result.eligible_observations != int(np.count_nonzero(panel.eligible[start:]))
        or result.valid_observations != int(np.count_nonzero(np.isfinite(result.values)))
        or np.any(np.isfinite(result.values) & ~panel.eligible[start:])
        or type(result.work_units) is not int
        or not 0 <= result.work_units <= MAX_WORK
    ):
        raise ValueError("transformation requires consistent raw values and coverage")
    output = np.full(expected, np.nan, dtype=np.float64)
    outcomes: list[Outcome] = []
    work = result.work_units
    for row, original in enumerate(result.values):
        index = start + row
        valid = panel.eligible[index] & np.isfinite(original)
        if exposures is not None:
            if policy.industry:
                valid &= np.asarray([value is not None for value in exposures.industry[index]])
            if policy.log_size:
                valid &= np.isfinite(exposures.market_cap[index])
            if policy.beta:
                valid &= np.isfinite(exposures.beta[index])
        count = int(np.count_nonzero(valid))
        work += len(panel.securities) + count * max(1, count.bit_length())
        if work > MAX_WORK:
            raise ValueError("cross-sectional transformation work budget")
        if count < policy.minimum_observations:
            outcomes.append("insufficient")
            continue
        values = _clip(original[valid], policy.winsor_tail_bps)
        if exposures is not None:
            design = _design(exposures, policy, index, valid)
            columns = design.shape[1]
            work += count * columns * columns
            if work > MAX_WORK:
                raise ValueError("cross-sectional transformation work budget")
            if count <= columns:
                outcomes.append("insufficient")
                continue
            scale = float(np.max(np.abs(values))) or 1.0
            scaled = values / scale
            try:
                coefficients, _, rank, _ = np.linalg.lstsq(design, scaled, rcond=RCOND)
            except np.linalg.LinAlgError as error:
                raise ValueError("neutralization solver failed") from error
            if rank != columns:
                outcomes.append("rank_deficient")
                continue
            residual = scaled - design @ coefficients
            if float(np.linalg.norm(residual)) <= RCOND * float(np.linalg.norm(scaled)):
                residual = np.zeros_like(residual)
            with np.errstate(over="raise", invalid="raise"):
                try:
                    values = residual if policy.standardize else residual * scale
                except FloatingPointError as error:
                    raise ValueError("neutralization residual exceeds binary64") from error
        if policy.standardize:
            centered = _centered_unit(values)
            deviation = math.sqrt(float(np.dot(centered, centered)) / (count - 1))
            if deviation == 0:
                outcomes.append("constant")
                continue
            values = centered / deviation
        if not np.isfinite(values).all():
            raise ValueError("non-finite cross-sectional result")
        output[row, valid] = values
        outcomes.append("ok")
    return Transformed(
        replace(
            result,
            values=_frozen(output),
            valid_observations=int(np.count_nonzero(np.isfinite(output))),
            work_units=work,
        ),
        result.valid_observations,
        tuple(outcomes),
    )
