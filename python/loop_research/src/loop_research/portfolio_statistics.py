"""Statistics derived from reconstructed ledgers and causal factor/exposure inputs."""

import csv
import io
import math
from collections import defaultdict
from collections.abc import Callable
from dataclasses import dataclass
from decimal import ROUND_HALF_EVEN, Context, Decimal, localcontext

import numpy as np
from loop_protocol.canonical import FactorDirection
from numpy.typing import NDArray

from loop_research.backtest import PortfolioReplay
from loop_research.build_identity import canonical_bytes
from loop_research.portfolio import Session, _Table, decimal_text
from loop_research.statistics_kernels import average_ranks, correlation, mean_test, sharpe
from loop_research.statistics_models import StatisticsPolicy, available, unavailable


@dataclass(frozen=True, slots=True)
class PortfolioStatistics:
    """Inspectable outputs and the complete chronological NAV-return vector."""

    artifacts: dict[str, bytes]
    returns: NDArray[np.float64]
    dates: tuple[str, ...]
    mean_p: float | None


def _rows(content: bytes) -> list[dict[str, str]]:
    return list(csv.DictReader(io.StringIO(content.decode("ascii"), newline=""), strict=True))


def _number(value: float | None) -> str:
    return "" if value is None or not math.isfinite(value) else format(value, ".17g")


def cross_sections(
    sessions: tuple[Session, ...],
    direction: FactorDirection,
    policy: StatisticsPolicy,
    check: Callable[[], None],
) -> tuple[bytes, dict[str, object]]:
    """No retrospective eligibility, label imputation or pairwise deletion."""
    groups = policy.groups
    table = _Table(
        (
            "decision_session",
            "label_session",
            "status",
            "signals",
            "labels",
            "ic",
            "rank_ic",
            *(f"group_{index + 1}" for index in range(groups)),
            "spread",
            "monotonicity",
        )
    )
    series: dict[str, list[float]] = {
        key: []
        for key in (
            "ic",
            "rank_ic",
            *(f"group_{index + 1}" for index in range(groups)),
            "spread",
            "monotonicity",
        )
    }
    orientation = 1 if direction == FactorDirection.HIGHER_IS_BETTER else -1
    for index, session in enumerate(sessions):
        check()
        selected = [
            column
            for column, row in enumerate(session.observations)
            if row.eligible and row.factor is not None
        ]
        status = "available"
        labels: list[float] = []
        if index == len(sessions) - 1:
            status = "no_forward_session"
        else:
            for column in selected:
                row = sessions[index + 1].observations[column]
                if row.opening is not None and row.closing is not None:
                    with localcontext(Context(prec=80, rounding=ROUND_HALF_EVEN)):
                        labels.append(float(row.closing / row.opening - 1))
            if len(labels) != len(selected):
                status = "missing_forward_prices"
            elif len(selected) < policy.minimum_cross_section:
                status = "insufficient_cross_section"
        values = {key: math.nan for key in series}
        if status == "available":
            signals = np.array(
                [orientation * float(session.observations[column].factor) for column in selected],  # type: ignore[arg-type]
                dtype=np.float64,
            )
            returns = np.array(labels, dtype=np.float64)
            ic = correlation(signals, returns)
            rank_ic = correlation(average_ranks(signals), average_ranks(returns))
            values["ic"] = ic.value if ic.value is not None else math.nan
            values["rank_ic"] = rank_ic.value if rank_ic.value is not None else math.nan
            if np.all(signals == signals[0]):
                status = "constant_signal"
            else:
                # Source securities are ordered stable IDs. Stable sorting retains
                # that declared tie break for group membership, not for IC ranks.
                order = np.argsort(signals, kind="stable")
                grouped = [
                    float(np.mean(returns[group])) for group in np.array_split(order, groups)
                ]
                for group, value in enumerate(grouped):
                    values[f"group_{group + 1}"] = value
                values["spread"] = grouped[-1] - grouped[0]
                monotonicity = (
                    correlation(
                        average_ranks(np.arange(groups, dtype=np.float64)),
                        average_ranks(np.array(grouped)),
                    )
                    if groups >= 3
                    else None
                )
                values["monotonicity"] = (
                    monotonicity.value
                    if monotonicity is not None and monotonicity.value is not None
                    else math.nan
                )
                if ic.value is None:
                    status = "constant_labels"
        for key, value in values.items():
            if index < len(sessions) - 1:
                series[key].append(value)
        table.append(
            session.day.isoformat(),
            sessions[index + 1].day.isoformat() if index + 1 < len(sessions) else "",
            status,
            len(selected),
            len(labels),
            *(_number(value) for value in values.values()),
        )
    summary: dict[str, object] = {
        "label": "next-session-raw-open-to-close.1",
        "group_basis": "equal-count-worst-to-best-stable-security-ties",
        "group_returns": "arithmetic-intraday-diagnostics-before-costs",
    }
    for key, observations in series.items():
        summary[key] = {
            name: metric.model_dump()
            for name, metric in mean_test(
                observations, minimum=policy.minimum_sessions, lags=policy.hac_lags
            ).items()
        }
    return table.content(), summary


def _exposures(
    replay: PortfolioReplay, positions: list[dict[str, str]], nav: Decimal, index: int
) -> tuple[float | None, float | None, dict[str, float] | None]:
    held = [row for row in positions if Decimal(row["market_value_usd"]) != 0]
    if not held:
        return 0.0, 0.0, {}
    exposure = replay.computed.loaded.exposures
    if exposure is None:
        return None, None, None
    day = replay.sessions[index].day
    row_index = exposure.sessions.index(day)
    columns = {security: column for column, security in enumerate(exposure.securities)}
    beta = size = 0.0
    missing_beta = missing_size = missing_industry = False
    industries: dict[str, float] = defaultdict(float)
    for row in held:
        column = columns[row["security_id"]]
        weight = float(Decimal(row["market_value_usd"]) / nav)
        b_value = float(exposure.beta[row_index, column])
        market_cap = float(exposure.market_cap[row_index, column])
        industry = exposure.industry[row_index][column]
        if math.isfinite(b_value):
            beta += weight * b_value
        else:
            missing_beta = True
        if math.isfinite(market_cap) and market_cap > 0:
            size += weight * math.log(market_cap)
        else:
            missing_size = True
        if industry is not None:
            industries[industry] += weight
        else:
            missing_industry = True
    return (
        None if missing_beta else beta,
        None if missing_size else size,
        None if missing_industry else dict(industries),
    )


def summarize(
    replay: PortfolioReplay, policy: StatisticsPolicy, check: Callable[[], None]
) -> PortfolioStatistics:
    """Use verified CSVs, never imported performance metrics or delta-NAV PnL."""
    with localcontext(Context(prec=80, rounding=ROUND_HALF_EVEN)):
        return _summarize(replay, policy, check)


def _summarize(
    replay: PortfolioReplay, policy: StatisticsPolicy, check: Callable[[], None]
) -> PortfolioStatistics:
    nav_rows = _rows(replay.artifacts["nav"])
    positions: dict[str, list[dict[str, str]]] = defaultdict(list)
    fills: dict[str, Decimal] = defaultdict(Decimal)
    for row in _rows(replay.artifacts["positions"]):
        positions[row["session"]].append(row)
    for row in _rows(replay.artifacts["fills"]):
        fills[row["session"]] += abs(Decimal(row["shares"]) * Decimal(row["price_usd"]))
    table = _Table(
        (
            "session",
            "nav_usd",
            "simple_return",
            "one_way_turnover",
            "drawdown",
            "gross_exposure",
            "net_exposure",
            "beta_exposure",
            "log_size_exposure",
            "industry_status",
        )
    )
    sectors = _Table(("session", "industry", "net_nav_weight"))
    navs = [Decimal(row["nav_usd"]) for row in nav_rows]
    peak = navs[0]
    returns: list[float] = []
    turnover = Decimal(0)
    max_drawdown = Decimal(0)
    beta_values, size_values = [], []
    for index, row in enumerate(nav_rows):
        check()
        day = row["session"]
        nav = navs[index]
        previous = navs[index - 1] if index else nav
        value = float(nav / previous - 1) if index else None
        if value is not None:
            returns.append(value)
        traded = fills[day] / previous / 2
        turnover += traded
        peak = max(peak, nav)
        drawdown = 1 - nav / peak
        max_drawdown = max(drawdown, max_drawdown)
        marks = [Decimal(position["market_value_usd"]) for position in positions[day]]
        gross = sum(map(abs, marks), Decimal(0)) / nav
        net = sum(marks, Decimal(0)) / nav
        beta, size, industries = _exposures(replay, positions[day], nav, index)
        beta_values.append(beta if beta is not None else math.nan)
        size_values.append(size if size is not None else math.nan)
        if industries is not None:
            for industry, weight in sorted(industries.items()):
                sectors.append(day, industry, _number(weight))
        table.append(
            day,
            decimal_text(nav),
            _number(value),
            decimal_text(traded),
            decimal_text(drawdown),
            decimal_text(gross),
            decimal_text(net),
            _number(beta),
            _number(size),
            "available" if industries is not None else "missing_held_exposures",
        )
    result = mean_test(returns, minimum=policy.minimum_sessions, lags=policy.hac_lags)
    ratio = sharpe(returns, policy.minimum_sessions)
    annualized = (
        available(ratio.value * math.sqrt(252), len(returns))
        if ratio.value is not None
        else unavailable(ratio.reason or "unavailable", len(returns))
    )
    cross_bytes, cross_summary = cross_sections(
        replay.sessions, replay.computed.factor.spec.direction, policy, check
    )
    summary = {
        "schema": "loop.statistics-summary/v1",
        "factor_spec_id": replay.receipt.factor_spec_id,
        "quality": replay.receipt.quality,
        "production_eligible": False,
        "policy": policy.model_dump(),
        "observed_sessions": len(navs),
        "return_observations": len(returns),
        "benchmark": "zero-daily-return",
        "sharpe_annualization": "sqrt(252)-descriptive-no-serial-correction",
        "mean_inference": "bartlett-hac-normal-two-sided-95pct",
        "total_return": available(float(navs[-1] / navs[0] - 1), len(returns)).model_dump(),
        "maximum_drawdown": available(float(max_drawdown), len(navs)).model_dump(),
        "one_way_turnover": available(float(turnover), len(navs)).model_dump(),
        "sharpe_daily": ratio.model_dump(),
        "sharpe_annualized": annualized.model_dump(),
        "return_mean": {name: metric.model_dump() for name, metric in result.items()},
        "beta_exposure": {
            name: metric.model_dump()
            for name, metric in mean_test(
                beta_values, minimum=policy.minimum_sessions, lags=policy.hac_lags
            ).items()
        },
        "log_size_exposure": {
            name: metric.model_dump()
            for name, metric in mean_test(
                size_values, minimum=policy.minimum_sessions, lags=policy.hac_lags
            ).items()
        },
        "cross_section": cross_summary,
    }
    return PortfolioStatistics(
        {
            "summary": canonical_bytes(summary),
            "cross_sections": cross_bytes,
            "portfolio": table.content(),
            "exposures": sectors.content(),
        },
        np.array(returns, dtype=np.float64),
        tuple(row["session"] for row in nav_rows[1:]),
        result["p_value"].value,
    )
