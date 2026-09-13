"""Bounded, causal evaluation of canonical factors on exactly aligned panels."""

from __future__ import annotations

import math
import re
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import date
from itertools import pairwise
from types import MappingProxyType

import numpy as np
from loop_protocol.canonical import (
    AstNode,
    CallNode,
    CanonicalFactorSpec,
    DecimalNode,
    FieldNode,
    parse_canonical_factor_spec,
)
from numpy.typing import NDArray

from loop_research.numerics import _centered_unit, adjusted_skew
from loop_research.operators import BINARY, LAGGED, ROLLING, VERSION, operator_registry

MAX_CELLS = 2_000_000
MAX_WORK = 50_000_000
_SECURITY = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}\Z", re.ASCII)


def _frozen(values: NDArray[np.float64]) -> NDArray[np.float64]:
    return np.frombuffer(values.tobytes(order="C"), dtype=np.float64).reshape(values.shape)


@dataclass(frozen=True, slots=True)
class Panel:
    """One immutable session/security grid; no implicit joins or missing-row fill.

    The data loader must additionally verify calendar completeness, sample
    authority and per-observation time visibility. These arrays alone prove
    neither market-data provenance nor point-in-time correctness.
    """

    sessions: tuple[date, ...]
    securities: tuple[str, ...]
    fields: Mapping[str, NDArray[np.float64]]
    eligible: NDArray[np.bool_]

    def __post_init__(self) -> None:
        if (
            not isinstance(self.sessions, tuple)
            or not isinstance(self.securities, tuple)
            or not self.sessions
            or not self.securities
            or len(self.sessions) * len(self.securities) > MAX_CELLS
            or any(type(value) is not date for value in self.sessions)
            or any(left >= right for left, right in pairwise(self.sessions))
            or any(
                type(value) is not str or not _SECURITY.fullmatch(value)
                for value in self.securities
            )
            or tuple(sorted(set(self.securities))) != self.securities
        ):
            raise ValueError("panel requires bounded, unique and ordered session/security axes")
        shape = (len(self.sessions), len(self.securities))
        if (
            not isinstance(self.eligible, np.ndarray)
            or isinstance(self.eligible, np.ma.MaskedArray)
            or self.eligible.dtype != np.bool_
            or self.eligible.shape != shape
            or not isinstance(self.fields, Mapping)
            or not self.fields
            or len(self.fields) > 32
        ):
            raise ValueError("panel requires explicit eligibility on the same axes")
        eligible = np.frombuffer(self.eligible.tobytes(order="C"), dtype=np.bool_).reshape(shape)
        fields = {}
        for name, values in self.fields.items():
            operator_registry().require_field(name)
            if (
                not isinstance(values, np.ndarray)
                or isinstance(values, np.ma.MaskedArray)
                or values.dtype != np.float64
                or values.shape != shape
                or np.isinf(values).any()
            ):
                raise ValueError("panel fields require finite-or-NaN binary64 on the same axes")
            fields[name] = _frozen(np.where(eligible, values, np.nan))
        object.__setattr__(self, "fields", MappingProxyType(fields))
        object.__setattr__(self, "eligible", eligible)


@dataclass(frozen=True, slots=True)
class Evaluation:
    """Raw factor values and exact coverage counts, never a performance claim."""

    factor_spec_id: str
    expression_id: str
    operator_registry_sha256: str
    evaluation_start: date
    values: NDArray[np.float64]
    eligible_observations: int
    valid_observations: int
    work_units: int


@dataclass(slots=True)
class _Budget:
    used: int = 0

    def charge(self, work: int) -> None:
        self.used += work
        if self.used > MAX_WORK:
            raise ValueError("factor evaluation work budget exhausted")


def _mean(values: NDArray[np.float64]) -> float:
    try:
        return math.fsum(float(value) / values.size for value in values)
    except OverflowError:
        return math.nan


def _std(values: NDArray[np.float64]) -> float:
    if values.size < 2:
        return math.nan
    # Anchoring preserves small deviations at large offsets. Scale opposite
    # extremes before subtraction when their finite difference would overflow.
    with np.errstate(over="ignore", invalid="ignore"):
        shifted = values - values[0]
        multiplier = 1.0
        if not np.isfinite(shifted).all():
            multiplier = float(np.max(np.abs(values)))
            shifted = values / multiplier - values[0] / multiplier
        scale = float(np.max(np.abs(shifted)))
        if scale == 0:
            return 0.0
        unit = shifted / scale
        centered = unit - _mean(unit)
        result = math.sqrt(float(np.dot(centered, centered)) / (values.size - 1))
        result *= scale
        result *= multiplier
    return result if math.isfinite(result) else math.nan


def _rank_last(window: NDArray[np.float64], valid: NDArray[np.float64]) -> float:
    target = float(window[-1])
    if math.isnan(target):
        return math.nan
    if valid.size == 1 or np.min(valid) == np.max(valid):
        return 0.5
    # Stable ascending order places the last observation after all equal peers.
    return (int(np.count_nonzero(valid <= target)) - 1) / (valid.size - 1)


def _rolling(
    name: str, values: NDArray[np.float64], width: int, minimum: int
) -> NDArray[np.float64]:
    required = 3 if name == "skew" else 2 if name == "std" else 1
    if not required <= minimum <= width <= 4096:
        raise ValueError("rolling width/minimum violates the operator contract")
    output = np.full(values.shape, np.nan, dtype=np.float64)
    for column in range(values.shape[1]):
        for end in range(values.shape[0]):
            window = values[max(0, end + 1 - width) : end + 1, column]
            valid = window[~np.isnan(window)]
            if valid.size < minimum:
                continue
            match name:
                case "ma":
                    result = _mean(valid)
                case "std":
                    result = _std(valid)
                case "min":
                    result = float(np.min(valid))
                case "max":
                    result = float(np.max(valid))
                case "skew":
                    result = adjusted_skew(valid)
                case "rank_ts":
                    result = _rank_last(window, valid)
                case _:
                    raise ValueError("unimplemented rolling operator")
            output[end, column] = result
    return output


def _cross_section(name: str, values: NDArray[np.float64]) -> NDArray[np.float64]:
    output = np.full(values.shape, np.nan, dtype=np.float64)
    for index, row in enumerate(values):
        present = ~np.isnan(row)
        valid = row[present]
        if not valid.size:
            continue
        if name == "rank_cs":
            _, inverse, counts = np.unique(valid, return_inverse=True, return_counts=True)
            upper = np.cumsum(counts)
            ranks = (upper - (counts - 1) / 2) / valid.size
            output[index, present] = ranks[inverse]
        elif name == "zscore":
            if valid.size < 2:
                continue
            centered = _centered_unit(valid)
            deviation = math.sqrt(float(np.dot(centered, centered)) / (valid.size - 1))
            if deviation != 0:
                output[index, present] = centered / deviation
        else:
            raise ValueError("unimplemented cross-sectional operator")
    return output


def _count(node: AstNode) -> int:
    if not isinstance(node, DecimalNode):
        raise ValueError("operator count must be a canonical integer literal")
    return int(node.value)


def _evaluate(node: AstNode, panel: Panel, budget: _Budget) -> NDArray[np.float64]:
    budget.charge(panel.eligible.size)
    if isinstance(node, FieldNode):
        try:
            return panel.fields[node.field]
        except KeyError as error:
            raise ValueError("factor field is absent from the authorized panel") from error
    if not isinstance(node, CallNode) or node.operator_version != VERSION:
        raise ValueError("factor must use an implemented series operator")
    name = node.operator
    values = _evaluate(node.arguments[0], panel, budget)
    if name in ROLLING:
        width = _count(node.arguments[1])
        minimum = _count(node.arguments[2])
        budget.charge(values.size * width)
        result = _rolling(name, values, width, minimum)
    elif name in LAGGED:
        width = _count(node.arguments[1])
        result = np.full(values.shape, np.nan, dtype=np.float64)
        with np.errstate(over="ignore", invalid="ignore", divide="ignore"):
            if width < len(values):
                result[width:] = (
                    values[width:] / values[:-width] - 1
                    if name == "roc"
                    else values[width:] - values[:-width]
                )
    elif name in BINARY:
        other = _evaluate(node.arguments[1], panel, budget)
        with np.errstate(over="ignore", invalid="ignore", divide="ignore"):
            match name:
                case "add":
                    result = values + other
                case "sub":
                    result = values - other
                case "mul":
                    result = values * other
                case "div":
                    result = values / other
                case _:
                    raise ValueError("unimplemented binary operator")
    else:
        if name == "rank_cs":
            budget.charge(values.size * max(1, math.ceil(math.log2(values.shape[1]))))
        result = _cross_section(name, values)
    return np.where(panel.eligible & np.isfinite(result), result, np.nan)


def evaluate(factor: CanonicalFactorSpec, panel: Panel, *, evaluation_start: date) -> Evaluation:
    """Reparse and execute exactly the bound canonical tree, without I/O.

    Values before the explicit evaluation boundary are warmup and are excluded
    from coverage and returned values. Direction is never selected or changed.
    The worker owns manifest/lease checks before and after this pure function.
    """
    if type(evaluation_start) is not date or evaluation_start not in panel.sessions:
        raise ValueError("evaluation boundary must be an exact panel session")
    bound = parse_canonical_factor_spec(
        factor.canonical_bytes,
        factor.factor_spec_id,
        factor.expression.canonical_bytes,
        operator_registry(),
    )
    budget = _Budget()
    values = _evaluate(bound.expression.ast, panel, budget)
    start = panel.sessions.index(evaluation_start)
    return Evaluation(
        bound.factor_spec_id,
        bound.expression.expression_id,
        bound.spec.operator_registry_sha256,
        evaluation_start,
        _frozen(values[start:]),
        int(np.count_nonzero(panel.eligible[start:])),
        int(np.count_nonzero(np.isfinite(values[start:]) & panel.eligible[start:])),
        budget.used,
    )
