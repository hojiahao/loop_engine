# -*- coding: utf-8 -*-
import numpy as np
import pytest

from backtest.returns import nav_to_returns, pairwise_finite_corr


# Scenario: nav to returns uses previous nav denominator.
def test_nav_previous():
    nav = [1.0, 1.10, 1.21]
    assert nav_to_returns(nav).tolist() == pytest.approx([0.10, 0.10])
    assert np.diff(nav).tolist() == pytest.approx([0.10, 0.11])


# Scenario: nav to returns rejects invalid denominator and nonfinite.
def test_nav_invalid():
    with pytest.raises(ValueError, match="zero denominator"):
        nav_to_returns([1.0, 0.0, 1.0])
    with pytest.raises(ValueError, match="non-finite"):
        nav_to_returns([1.0, np.nan])


# Scenario: pairwise corr masks only pairwise nonfinite observations.
def test_pairwise_corr():
    a = [1.0, 2.0, np.nan, 4.0, 5.0, 6.0]
    b = [2.0, 4.0, 100.0, 8.0, 10.0, 12.0]
    assert pairwise_finite_corr(a, b) == pytest.approx(1.0)
