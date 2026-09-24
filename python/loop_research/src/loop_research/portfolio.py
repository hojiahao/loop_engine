"""Deterministic long-only, next-session-open accounting reference (ADR 0029)."""

import csv
import io
import math
import re
from collections.abc import Callable
from dataclasses import dataclass
from datetime import date
from decimal import ROUND_CEILING, ROUND_FLOOR, ROUND_HALF_EVEN, Context, Decimal, localcontext

from loop_protocol.canonical import FactorDirection

from loop_research.backtest_models import (
    MAX_REPLAY_BYTES,
    MAX_REPLAY_CELLS,
    PortfolioPolicy,
    money,
)

USD_UNIT = Decimal("0.00000001")
MAX_SHARES = 10**12


def decimal_text(value: Decimal) -> str:
    """Emit a stable ordinary decimal without exponents, redundant zeros or -0."""
    if value == 0:
        return "0"
    result = format(value, "f")
    return result.rstrip("0").rstrip(".") if "." in result else result


@dataclass(frozen=True, slots=True)
class Observation:
    """One complete grid cell; missing opening observations never imply a fill."""

    security_id: str
    eligible: bool
    factor: float | None
    open_at_ms: int | None
    opening: Decimal | None
    close_known_at_ms: int | None
    closing: Decimal | None


@dataclass(frozen=True, slots=True)
class Session:
    """Prepared session inputs; the IO boundary checks the actual exchange calendar."""

    day: date
    decision_ms: int
    observations: tuple[Observation, ...]


@dataclass(frozen=True, slots=True)
class Ledger:
    """Complete deterministic CSV artifacts and bounded accounting summaries."""

    artifacts: dict[str, bytes]
    orders: int
    fills: int
    ending_nav: Decimal


class _Table:
    def __init__(self, header: tuple[str, ...]) -> None:
        self.stream = io.StringIO(newline="")
        self.writer = csv.writer(self.stream, lineterminator="\n")
        self.writer.writerow(header)

    def append(self, *values: str | int | None) -> None:
        self.writer.writerow(values)
        if self.stream.tell() > MAX_REPLAY_BYTES:
            raise ValueError("portfolio artifact exceeds byte budget")

    def content(self) -> bytes:
        return self.stream.getvalue().encode("ascii")


def _validate(sessions: tuple[Session, ...], direction: FactorDirection) -> tuple[str, ...]:
    if not isinstance(direction, FactorDirection) or not 2 <= len(sessions) <= 8192:
        raise ValueError("portfolio requires a frozen direction and at least two sessions")
    securities = tuple(row.security_id for row in sessions[0].observations)
    if (
        not securities
        or tuple(sorted(set(securities))) != securities
        or len(securities) * len(sessions) > MAX_REPLAY_CELLS
        or any(
            not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}", value) for value in securities
        )
    ):
        raise ValueError("portfolio security grid is unordered or exceeds cell budget")
    previous = None
    for session in sessions:
        if type(session.day) is not date or type(session.decision_ms) is not int:
            raise ValueError("portfolio session requires an explicit date and clock")
        if previous is not None and (
            session.day <= previous.day or session.decision_ms <= previous.decision_ms
        ):
            raise ValueError("portfolio session clock regression")
        if tuple(row.security_id for row in session.observations) != securities:
            raise ValueError("portfolio security axes differ")
        for row in session.observations:
            if any(
                value is not None and (type(value) is not int or not 0 < value < 2**53)
                for value in (row.open_at_ms, row.close_known_at_ms)
            ):
                raise ValueError("portfolio observation clock bounds")
            if type(row.eligible) is not bool or (
                row.factor is not None
                and (type(row.factor) is not float or not math.isfinite(row.factor))
            ):
                raise ValueError("portfolio signals must be finite or explicitly missing")
            if (row.opening is None) != (row.open_at_ms is None) or (row.closing is None) != (
                row.close_known_at_ms is None
            ):
                raise ValueError("portfolio price and visibility must be paired")
            for price in (row.opening, row.closing):
                if price is not None and (
                    money(str(price), positive=True, maximum="1000000000") != price
                    or price < Decimal("0.000001")
                ):
                    raise ValueError("portfolio price bounds")
            if row.close_known_at_ms is not None and row.close_known_at_ms > session.decision_ms:
                raise ValueError("closing price was unavailable at decision time")
            if row.open_at_ms is not None and (
                row.open_at_ms >= session.decision_ms
                or (previous is not None and row.open_at_ms <= previous.decision_ms)
            ):
                raise ValueError("opening event is outside its execution interval")
        previous = session
    return securities


def _rank(row: Observation, direction: FactorDirection) -> tuple[float, str]:
    value = row.factor
    if value is None:
        raise ValueError("missing signal cannot be ranked")
    return (-value if direction == FactorDirection.HIGHER_IS_BETTER else value, row.security_id)


def replay(
    sessions: tuple[Session, ...],
    policy: PortfolioPolicy,
    direction: FactorDirection,
    *,
    check_budget: Callable[[], object] = lambda: None,
) -> Ledger:
    """Replay fixed-share orders; the next opening never influences target sizing.

    USD arithmetic uses an isolated 80-digit Decimal context. Modeled buy/sell
    prices round adversely to 1e-8 USD; fees and all cash/valuation entries are
    exact at that precision. No external cash flows, financing, short positions,
    corporate actions, liquidity guarantee or final liquidation is implied.
    """
    policy = PortfolioPolicy.model_validate(policy)
    securities = _validate(sessions, direction)
    with localcontext(Context(prec=80, rounding=ROUND_HALF_EVEN)):
        return _replay(sessions, securities, policy, direction, check_budget)


def _replay(
    sessions: tuple[Session, ...],
    securities: tuple[str, ...],
    policy: PortfolioPolicy,
    direction: FactorDirection,
    check_budget: Callable[[], object],
) -> Ledger:
    targets = _Table(("decision_session", "execute_session", "security_id", "shares"))
    orders = _Table(
        (
            "order_id",
            "decision_session",
            "execute_session",
            "security_id",
            "side",
            "shares",
            "filled_shares",
            "status",
        )
    )
    fills = _Table(("order_id", "session", "security_id", "at_ms", "side", "shares", "price_usd"))
    positions = _Table(("session", "security_id", "shares", "mark_usd", "market_value_usd"))
    navs = _Table(("session", "cash_usd", "market_value_usd", "nav_usd"))
    returns = _Table(("session", "previous_nav_usd", "nav_usd", "simple_return"))
    costs = _Table(("order_id", "session", "commission_usd", "spread_cost_usd", "cash_delta_usd"))
    cash = Decimal(policy.initial_cash_usd)
    holdings = dict.fromkeys(securities, 0)
    pending: dict[str, tuple[str, int]] = {}
    fee_per_share = Decimal(policy.commission_per_share_usd)
    minimum_fee = Decimal(policy.minimum_commission_usd)
    spread = Decimal(policy.half_spread_bps) / 10000
    previous_nav: Decimal | None = None
    order_count = fill_count = 0
    for index, session in enumerate(sessions):
        check_budget()
        # Actual event order precedes the same-time sell/buy and stable-ID tie rules.
        # A sale at 09:35 cannot finance a buy observed at 09:30.
        events = sorted(
            (row for row in session.observations if row.security_id in pending),
            key=lambda row: (
                row.open_at_ms if row.open_at_ms is not None else session.decision_ms,
                pending[row.security_id][1] > 0,
                row.security_id,
            ),
        )
        for row in events:
            check_budget()
            order_id, signed_shares = pending[row.security_id]
            side = "buy" if signed_shares > 0 else "sell"
            requested = abs(signed_shares)
            quantity = 0
            status = "missing_open"
            if row.opening is not None:
                price = (row.opening * (1 + spread if side == "buy" else 1 - spread)).quantize(
                    USD_UNIT, rounding=ROUND_CEILING if side == "buy" else ROUND_FLOOR
                )
                quantity = requested
                if side == "buy":
                    affordable = min(
                        (cash - minimum_fee) / price,
                        cash / (price + fee_per_share),
                    )
                    lots = max(
                        0,
                        int((affordable / policy.lot_size).to_integral_value(rounding=ROUND_FLOOR)),
                    )
                    quantity = min(quantity, lots * policy.lot_size)
                fee = max(minimum_fee, quantity * fee_per_share) if quantity else Decimal(0)
                notional = quantity * price
                if side == "sell" and cash + notional < fee:
                    quantity = 0
                status = (
                    "filled" if quantity == requested else "partial_cash" if quantity else "cash"
                )
                if quantity:
                    cash_delta = (-notional if side == "buy" else notional) - fee
                    cash += cash_delta
                    holdings[row.security_id] += quantity if side == "buy" else -quantity
                    if cash < 0 or holdings[row.security_id] < 0:
                        raise ValueError("portfolio violated its cash-only long constraint")
                    fills.append(
                        order_id,
                        session.day.isoformat(),
                        row.security_id,
                        row.open_at_ms,
                        side,
                        quantity,
                        decimal_text(price),
                    )
                    costs.append(
                        order_id,
                        session.day.isoformat(),
                        decimal_text(fee),
                        decimal_text(quantity * abs(price - row.opening)),
                        decimal_text(cash_delta),
                    )
                    fill_count += 1
            orders.append(
                order_id,
                sessions[index - 1].day.isoformat(),
                session.day.isoformat(),
                row.security_id,
                side,
                requested,
                quantity,
                status,
            )
        pending = {}
        market_value = Decimal(0)
        for row in session.observations:
            shares = holdings[row.security_id]
            if shares and row.closing is None:
                raise ValueError("held security has no visible closing mark")
            value = shares * row.closing if row.closing is not None else Decimal(0)
            market_value += value
            positions.append(
                session.day.isoformat(),
                row.security_id,
                shares,
                decimal_text(row.closing) if row.closing is not None else None,
                decimal_text(value),
            )
        nav = cash + market_value
        if nav <= 0 or nav > Decimal("1000000000000000000000000"):
            raise ValueError("portfolio NAV exceeds the supported solvent ledger bounds")
        navs.append(
            session.day.isoformat(),
            decimal_text(cash),
            decimal_text(market_value),
            decimal_text(nav),
        )
        # There is no observed prior NAV for the first session. Never invent a return.
        returns.append(
            session.day.isoformat(),
            decimal_text(previous_nav) if previous_nav else None,
            decimal_text(nav),
            decimal_text((nav / previous_nav - 1).quantize(Decimal("0.000000000000000001")))
            if previous_nav
            else None,
        )
        previous_nav = nav
        if index == len(sessions) - 1:
            continue
        selected = sorted(
            (row for row in session.observations if row.eligible and row.factor is not None),
            key=lambda row: _rank(row, direction),
        )[: policy.holdings]
        desired = dict.fromkeys(securities, 0)
        for row in selected:
            if row.closing is None:
                raise ValueError("selected security has no visible price for order sizing")
            lots = int(
                (nav / (len(selected) * row.closing * policy.lot_size)).to_integral_value(
                    rounding=ROUND_FLOOR
                )
            )
            shares = lots * policy.lot_size
            if shares > MAX_SHARES:
                raise ValueError("target shares exceed portfolio quantity budget")
            desired[row.security_id] = shares
        for security in securities:
            target = desired[security]
            targets.append(
                session.day.isoformat(), sessions[index + 1].day.isoformat(), security, target
            )
            difference = target - holdings[security]
            if difference:
                order_count += 1
                pending[security] = (f"order.{order_count:08d}", difference)
    tables = dict(
        targets=targets,
        orders=orders,
        fills=fills,
        positions=positions,
        nav=navs,
        returns=returns,
        costs=costs,
    )
    if sum(table.stream.tell() for table in tables.values()) > MAX_REPLAY_BYTES:
        raise ValueError("aggregate portfolio artifacts exceed byte budget")
    check_budget()
    return Ledger(
        {name: table.content() for name, table in tables.items()}, order_count, fill_count, nav
    )
