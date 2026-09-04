# -*- coding: utf-8 -*-
"""Return-series primitives shared by filtering, storage, and reporting."""
from __future__ import annotations

import numpy as np


def nav_to_returns(nav) -> np.ndarray:
    """Convert a NAV sequence to simple returns ``NAV[t] / NAV[t-1] - 1``.

    A return is undefined across a non-finite or zero prior NAV, so malformed
    inputs fail closed instead of silently contaminating correlation estimates.
    """
    values = np.asarray(nav, dtype=float)
    if values.ndim != 1:
        raise ValueError(f"NAV must be one-dimensional, got shape={values.shape}")
    if values.size < 2:
        return np.asarray([], dtype=float)
    if not np.isfinite(values).all():
        raise ValueError("NAV contains non-finite values")
    if np.any(values[:-1] == 0):
        raise ValueError("NAV contains a zero denominator")
    return values[1:] / values[:-1] - 1.0


def pairwise_finite_corr(a, b, *, min_observations: int = 5) -> float:
    """Pearson correlation on aligned trailing observations with finite masking."""
    left = np.asarray(a, dtype=float)
    right = np.asarray(b, dtype=float)
    if left.ndim != 1 or right.ndim != 1:
        raise ValueError("correlation inputs must be one-dimensional")
    n = min(left.size, right.size)
    if n < min_observations:
        return float("nan")
    left, right = left[-n:], right[-n:]
    valid = np.isfinite(left) & np.isfinite(right)
    if int(valid.sum()) < min_observations:
        return float("nan")
    left, right = left[valid], right[valid]
    if np.ptp(left) == 0 or np.ptp(right) == 0:
        return float("nan")
    return float(np.corrcoef(left, right)[0, 1])
