"""Source-timed action, borrow, financing and capacity accounting (ADR 0030)."""

import heapq
from collections.abc import Callable
from dataclasses import dataclass
from datetime import date
from decimal import ROUND_CEILING, ROUND_FLOOR, ROUND_HALF_EVEN, Context, Decimal, localcontext
from fractions import Fraction
from itertools import groupby

from loop_protocol.canonical import FactorDirection

from loop_research.backtest_models import MAX_REPLAY_BYTES
from loop_research.execution_inputs import MarketRow, MarketSession, milliseconds
from loop_research.market_models import Action, CashAction, ExecutionTerms, MarketPolicy, Split
from loop_research.portfolio import (
    MAX_SHARES,
    USD_UNIT,
    Ledger,
    _rank,
    _Table,
    _validate,
    decimal_text,
)


def _usd(value: Decimal, *, cost: bool = False) -> Decimal:
    return value.quantize(USD_UNIT, rounding=ROUND_CEILING if cost else ROUND_HALF_EVEN)


@dataclass(frozen=True, slots=True)
class _Quote:
    price: Decimal
    commission: Decimal
    sec_fee: Decimal
    taf_fee: Decimal
    spread: Decimal
    impact: Decimal

    @property
    def fees(self) -> Decimal:
        return self.commission + self.sec_fee + self.taf_fee


class _Account:
    """One bounded in-memory replay; publication and authority stay outside it."""

    def __init__(self, securities: tuple[str, ...], policy: MarketPolicy) -> None:
        self.policy = policy
        self.cash = Decimal(policy.initial_cash_usd)
        self.receivable = Decimal(0)
        self.payable = Decimal(0)
        self.holdings = dict.fromkeys(securities, 0)
        self.marks = dict.fromkeys(securities, Decimal(0))
        self.pending: dict[str, int] = {}
        self.claims: list[tuple[int, str, str, Decimal]] = []
        self.retired: set[str] = set()
        self.previous_terms: dict[str, ExecutionTerms] = {}
        self.order_count = self.fill_count = 0
        self.tables = {
            "targets": _Table(("decision_session", "execute_session", "security_id", "shares")),
            "orders": _Table(
                (
                    "order_id",
                    "decision_session",
                    "execute_session",
                    "security_id",
                    "side",
                    "shares",
                    "filled_shares",
                    "status",
                    "reason",
                )
            ),
            "fills": _Table(
                ("order_id", "session", "security_id", "at_ms", "side", "shares", "price_usd")
            ),
            "positions": _Table(
                ("session", "security_id", "shares", "mark_usd", "market_value_usd")
            ),
            "nav": _Table(
                (
                    "session",
                    "cash_usd",
                    "receivable_usd",
                    "market_value_usd",
                    "gross_value_usd",
                    "short_collateral_usd",
                    "nav_usd",
                )
            ),
            "returns": _Table(("session", "previous_nav_usd", "nav_usd", "simple_return")),
            "costs": _Table(
                (
                    "event_id",
                    "session",
                    "security_id",
                    "kind",
                    "cash_delta_usd",
                    "receivable_delta_usd",
                    "commission_usd",
                    "sec_fee_usd",
                    "taf_fee_usd",
                    "spread_cost_usd",
                    "impact_cost_usd",
                )
            ),
        }

    def flow(
        self,
        day: date,
        identity: str,
        security: str,
        kind: str,
        cash: Decimal = Decimal(0),
        receivable: Decimal = Decimal(0),
        commission: Decimal = Decimal(0),
        sec_fee: Decimal = Decimal(0),
        taf_fee: Decimal = Decimal(0),
        spread: Decimal = Decimal(0),
        impact: Decimal = Decimal(0),
    ) -> None:
        self.cash += cash
        self.receivable += receivable
        self.tables["costs"].append(
            identity,
            day.isoformat(),
            security,
            kind,
            *(
                decimal_text(value)
                for value in (cash, receivable, commission, sec_fee, taf_fee, spread, impact)
            ),
        )

    def collateral(self, security: str) -> Decimal:
        per_share = (
            self.marks[security] * self.policy.short_collateral_bps / 10000
        ).to_integral_value(rounding=ROUND_CEILING)
        return max(0, -self.holdings[security]) * per_share

    def valuation(self) -> tuple[Decimal, Decimal, Decimal, Decimal]:
        values = [shares * self.marks[security] for security, shares in self.holdings.items()]
        net, gross = sum(values, Decimal(0)), sum(map(abs, values), Decimal(0))
        collateral = sum((self.collateral(security) for security in self.holdings), Decimal(0))
        return self.cash + self.receivable + net, net, gross, collateral

    def finance(self, previous: date, current: date) -> None:
        days = (current - previous).days
        _, _, _, collateral = self.valuation()
        # Prior-close economic balances are held over the full ACT interval.
        # Short proceeds are collateralized; dividend liabilities are reserved.
        balance = self.cash - collateral + self.payable
        rate = self.policy.cash_credit_bps if balance >= 0 else self.policy.cash_debit_bps
        amount = abs(balance) * rate * days / (10000 * self.policy.day_count)
        interest = (
            amount.quantize(USD_UNIT, rounding=ROUND_FLOOR)
            if balance >= 0
            else -_usd(amount, cost=True)
        )
        if interest:
            self.flow(
                current, "finance." + previous.isoformat(), "", "cash_interest", cash=interest
            )
        for security, shares in self.holdings.items():
            if shares >= 0:
                continue
            terms = self.previous_terms.get(security)
            if terms is None:
                raise ValueError("short position lacks visible closing borrow terms")
            charge = _usd(
                self.collateral(security)
                * terms.borrow_rate_bps
                * days
                / (10000 * self.policy.day_count),
                cost=True,
            )
            self.flow(
                current, "borrow." + previous.isoformat(), security, "borrow_fee", cash=-charge
            )

    def pay(self, at: int, day: date) -> None:
        while self.claims and self.claims[0][0] <= at:
            _, identity, security, amount = heapq.heappop(self.claims)
            if amount < 0:
                self.payable -= amount
            self.flow(day, identity, security, "payment", cash=amount, receivable=-amount)

    def claim(self, action: Action, amount: Decimal, pay_ms: int, kind: str) -> None:
        if amount:
            if amount < 0:
                self.payable += amount
            self.flow(action.session, action.event_id, action.security_id, kind, receivable=amount)
            heapq.heappush(self.claims, (pay_ms, action.event_id, action.security_id, amount))

    def action(self, action: Action, previous: date | None) -> None:
        security = action.security_id
        if security in self.retired:
            raise ValueError("corporate action follows final delisting settlement")
        shares = self.holdings[security]
        if isinstance(action, Split):
            converted = Fraction(shares * action.numerator, action.denominator)
            whole = int(converted)  # Truncate toward zero, including short liabilities.
            if abs(whole) > MAX_SHARES:
                raise ValueError("split holdings exceed the quantity budget")
            fraction = converted - whole
            if fraction:
                if action.fraction_price_usd is None or action.pay_at is None:
                    raise ValueError("fractional split requires visible cash-in-lieu terms")
                amount = _usd(
                    Decimal(fraction.numerator)
                    * Decimal(action.fraction_price_usd)
                    / fraction.denominator
                )
                self.claim(action, amount, milliseconds(action.pay_at), "cash_in_lieu")
            self.holdings[security] = whole
            self.marks[security] *= Decimal(action.denominator) / action.numerator
            if security in self.pending:
                self.pending[security] = int(
                    Fraction(self.pending[security] * action.numerator, action.denominator)
                )
                if abs(self.pending[security]) > MAX_SHARES:
                    raise ValueError("split target exceeds the quantity budget")
            self.flow(action.session, action.event_id, security, "split")
        elif isinstance(action, CashAction):
            amount = _usd(shares * Decimal(action.amount_per_share_usd))
            self.claim(action, amount, milliseconds(action.pay_at), action.kind)
            if action.kind == "delisting":
                if security in self.pending and previous is not None:
                    difference = self.pending.pop(security) - shares
                    self.order(
                        action.session, previous, security, difference, 0, "delisted", "signal"
                    )
                self.holdings[security] = 0
                self.marks[security] = Decimal(0)
                self.retired.add(security)
                self.flow(action.session, action.event_id, security, "retired")
        else:
            raise ValueError("unsupported corporate action")

    def order(
        self,
        day: date,
        decision: date,
        security: str,
        signed: int,
        filled: int,
        status: str,
        reason: str,
    ) -> str:
        self.order_count += 1
        identity = f"order.{self.order_count:08d}"
        self.tables["orders"].append(
            identity,
            decision.isoformat(),
            day.isoformat(),
            security,
            "buy" if signed > 0 else "sell",
            abs(signed),
            filled,
            status,
            reason,
        )
        return identity

    def quote(self, row: MarketRow, signed: int) -> _Quote:
        opening = row.observation.opening
        if opening is None or row.auction_volume <= 0 or not signed:
            raise ValueError("fill requires an observed opening and positive capacity")
        quantity = abs(signed)
        spread = opening * self.policy.half_spread_bps / 10000
        impact = opening * self.policy.impact_bps * quantity / (10000 * row.auction_volume)
        price = (opening + (spread + impact) * (1 if signed > 0 else -1)).quantize(
            USD_UNIT, rounding=ROUND_CEILING if signed > 0 else ROUND_FLOOR
        )
        if price <= 0:
            raise ValueError("modeled execution price is nonpositive")
        fee = max(
            Decimal(self.policy.minimum_commission_usd),
            quantity * Decimal(self.policy.commission_per_share_usd),
        )
        terms = row.opening_terms
        if terms is None:
            raise ValueError("fill requires visible fee and availability terms")
        sec_fee = taf_fee = Decimal(0)
        if signed < 0:
            sec_fee = (
                quantity * price * Decimal(terms.sec_fee_usd_per_million) / 1_000_000
            ).quantize(Decimal("0.01"), rounding=ROUND_CEILING)
            taf_fee = min(
                quantity * Decimal(terms.taf_fee_usd_per_share), Decimal(terms.taf_fee_cap_usd)
            ).quantize(Decimal("0.01"), rounding=ROUND_CEILING)
        # Assign adverse price-rounding residuals to impact; never double charge.
        spread_cost = _usd(spread * quantity)
        impact_cost = abs(price - opening) * quantity - spread_cost
        return _Quote(price, fee, sec_fee, taf_fee, spread_cost, impact_cost)

    def affordable(self, row: MarketRow, signed: int) -> int:
        security = row.observation.security_id
        old = self.holdings[security]
        sign = 1 if signed > 0 else -1
        quantity = abs(signed)
        reduction = min(quantity, abs(old)) if old * sign < 0 else 0
        nav, _, gross, _ = self.valuation()
        # Unpaid positive entitlements are NAV assets, not financing collateral.
        nav -= self.receivable - self.payable
        mark = self.marks[security]

        def fits(amount: int) -> bool:
            if not amount:
                return True
            quote = self.quote(row, sign * amount)
            equity = nav + sign * amount * (mark - quote.price) - quote.fees
            exposure = gross + (abs(old + sign * amount) - abs(old)) * mark
            return equity > 0 and equity * 10000 >= exposure * self.policy.initial_margin_bps

        # Closing risk is allowed even during a margin breach; opening new risk
        # must satisfy initial margin. Final insolvency/maintenance still fails.
        lower, upper = reduction, quantity
        if lower and not fits(lower):
            return lower
        while lower < upper:
            middle = (lower + upper + 1) // 2
            if fits(middle):
                lower = middle
            else:
                upper = middle - 1
        return lower

    def open(self, session: MarketSession, row: MarketRow, previous: date | None) -> None:
        observation, terms = row.observation, row.opening_terms
        security = observation.security_id
        old = self.holdings[security]
        if security in self.retired:
            if observation.opening is not None or observation.eligible:
                raise ValueError("a cash-delisted security cannot resume trading or selection")
            return
        if old < 0 and terms is None:
            raise ValueError("short position lacks visible opening borrow terms")
        target = self.pending.pop(security, old)
        allowed = 0 if terms is None or terms.recalled else terms.borrow_limit
        recall = old < -allowed
        if recall:
            target = max(target, -allowed)
        signed = target - old
        if not signed:
            return
        requested, quantity = abs(signed), abs(signed)
        sign = 1 if signed > 0 else -1
        status = "filled"
        if observation.opening is None:
            quantity, status = 0, "missing_open"
        elif terms is None:
            quantity, status = 0, "missing_terms"
        elif not terms.tradable:
            quantity, status = 0, "untradable"
        else:
            capacity = row.auction_volume * self.policy.participation_bps // 10000
            if quantity > capacity:
                quantity, status = capacity, "capacity"
            if sign < 0:
                short_limit = allowed if terms.short_allowed else min(allowed, max(0, -old))
                borrow_quantity = max(0, old + short_limit)
                if quantity > borrow_quantity:
                    quantity, status = borrow_quantity, "borrow"
            if quantity:
                affordable = self.affordable(row, sign * quantity)
                if affordable < quantity:
                    quantity, status = affordable, "margin"
        reason = "borrow_recall" if recall else "signal"
        identity = self.order(
            session.base.day,
            session.base.day if recall else previous or session.base.day,
            security,
            signed,
            quantity,
            "partial_" + status if 0 < quantity < requested else status,
            reason,
        )
        if quantity:
            quote = self.quote(row, sign * quantity)
            self.holdings[security] += sign * quantity
            self.flow(
                session.base.day,
                identity,
                security,
                "trade",
                cash=-sign * quantity * quote.price - quote.fees,
                commission=quote.commission,
                sec_fee=quote.sec_fee,
                taf_fee=quote.taf_fee,
                spread=quote.spread,
                impact=quote.impact,
            )
            self.tables["fills"].append(
                identity,
                session.base.day.isoformat(),
                security,
                observation.open_at_ms,
                "buy" if sign > 0 else "sell",
                quantity,
                decimal_text(quote.price),
            )
            self.fill_count += 1
            if self.valuation()[0] <= 0:
                raise ValueError("portfolio insolvency after execution costs")
        if recall and self.holdings[security] < -allowed:
            raise ValueError("borrow recall cannot be completed within observed liquidity")

    def close(self, session: MarketSession, previous_nav: Decimal | None) -> Decimal:
        self.previous_terms = {}
        for row in session.rows:
            observation = row.observation
            security, shares = observation.security_id, self.holdings[observation.security_id]
            if shares and observation.closing is None:
                raise ValueError("held security has no visible closing mark")
            self.marks[security] = observation.closing or Decimal(0)
            if shares < 0:
                if row.closing_terms is None:
                    raise ValueError("short position lacks visible closing borrow terms")
                if row.closing_terms.recalled or row.closing_terms.borrow_limit < -shares:
                    raise ValueError(
                        "intraday borrow withdrawal requires a finer execution profile"
                    )
                self.previous_terms[security] = row.closing_terms
            self.tables["positions"].append(
                session.base.day.isoformat(),
                security,
                shares,
                decimal_text(observation.closing) if observation.closing is not None else None,
                decimal_text(shares * self.marks[security]),
            )
        nav, net, gross, collateral = self.valuation()
        if not 0 < nav <= Decimal("1000000000000000000000000"):
            raise ValueError("portfolio insolvency or NAV bound exceeded")
        if nav * 10000 < gross * self.policy.maintenance_margin_bps:
            raise ValueError("portfolio maintenance margin breached")
        self.tables["nav"].append(
            session.base.day.isoformat(),
            *(
                decimal_text(value)
                for value in (self.cash, self.receivable, net, gross, collateral, nav)
            ),
        )
        self.tables["returns"].append(
            session.base.day.isoformat(),
            decimal_text(previous_nav) if previous_nav else None,
            decimal_text(nav),
            decimal_text((nav / previous_nav - 1).quantize(Decimal("1e-18")))
            if previous_nav
            else None,
        )
        return nav

    def plan(
        self, session: MarketSession, following: date, nav: Decimal, direction: FactorDirection
    ) -> None:
        ranked = sorted(
            (
                row.observation
                for row in session.rows
                if row.observation.eligible
                and row.observation.factor is not None
                and row.observation.security_id not in self.retired
            ),
            key=lambda observation: _rank(observation, direction),
        )
        count = self.policy.holdings
        # If both legs are configured, insufficient breadth targets cash; never
        # select the same security for both sides or silently renormalize leverage.
        enough = (
            not (self.policy.long_weight_bps and self.policy.short_weight_bps)
            or len(ranked) >= 2 * count
        )
        long = ranked[:count] if self.policy.long_weight_bps and enough else []
        short = ranked[-count:] if self.policy.short_weight_bps and enough else []
        desired = dict.fromkeys(self.holdings, 0)
        for selected, weight, sign in (
            (long, self.policy.long_weight_bps, 1),
            (short, self.policy.short_weight_bps, -1),
        ):
            for row in selected:
                if row.closing is None:
                    raise ValueError("selected security has no visible sizing mark")
                lots = int(
                    (
                        nav * weight / (10000 * len(selected) * row.closing * self.policy.lot_size)
                    ).to_integral_value(rounding=ROUND_FLOOR)
                )
                shares = lots * self.policy.lot_size
                if shares > MAX_SHARES:
                    raise ValueError("target shares exceed the quantity budget")
                desired[row.security_id] = sign * shares
        self.pending = {}
        for security, target in desired.items():
            self.tables["targets"].append(
                session.base.day.isoformat(), following.isoformat(), security, target
            )
            if target != self.holdings[security]:
                self.pending[security] = target


def replay_market(
    sessions: tuple[MarketSession, ...],
    policy: MarketPolicy,
    direction: FactorDirection,
    *,
    check_budget: Callable[[], None] = lambda: None,
) -> Ledger:
    """Replay frozen development inputs; missing required evidence fails closed.

    Economic trade-date cash, ACT prior-close financing and explicit margin
    parameters are research assumptions, not a regulated brokerage account.
    Existing version-1 no-action ledgers retain their original implementation.
    """
    policy = MarketPolicy.model_validate(policy)
    securities = _validate(tuple(session.base for session in sessions), direction)
    with localcontext(Context(prec=80, rounding=ROUND_HALF_EVEN)):
        account = _Account(securities, policy)
        previous_nav = None
        for index, session in enumerate(sessions):
            check_budget()
            if tuple(row.observation for row in session.rows) != session.base.observations:
                raise ValueError("market observation axes differ")
            previous = sessions[index - 1].base.day if index else None
            if previous is not None:
                account.finance(previous, session.base.day)
            events: list[tuple[int, int, str, Action | MarketRow]] = [
                (milliseconds(action.effective_at), 0, action.security_id, action)
                for action in session.actions
            ]
            events.extend(
                (
                    row.observation.open_at_ms or session.scheduled_open_ms,
                    1,
                    row.observation.security_id,
                    row,
                )
                for row in session.rows
            )
            for at, group in groupby(
                sorted(events, key=lambda value: value[:3]), key=lambda value: value[0]
            ):
                check_budget()
                account.pay(at, session.base.day)
                openings = []
                for _, _, security, event in group:
                    if isinstance(event, Action):
                        account.action(event, previous)
                    else:
                        if event.observation.opening is not None:
                            account.marks[security] = event.observation.opening
                        openings.append(event)
                account.pay(at, session.base.day)
                if account.valuation()[0] <= 0:
                    raise ValueError("portfolio insolvency at an observed event")
                openings.sort(
                    key=lambda row: (
                        (
                            account.holdings[row.observation.security_id] < 0
                            and row.opening_terms is not None
                            and (
                                row.opening_terms.recalled
                                or row.opening_terms.borrow_limit
                                < -account.holdings[row.observation.security_id]
                            )
                        )
                        or account.pending.get(
                            row.observation.security_id,
                            account.holdings[row.observation.security_id],
                        )
                        > account.holdings[row.observation.security_id],
                        row.observation.security_id,
                    )
                )
                for row in openings:
                    check_budget()
                    account.open(session, row, previous)
            account.pay(session.base.decision_ms, session.base.day)
            previous_nav = account.close(session, previous_nav)
            if index < len(sessions) - 1:
                account.plan(session, sessions[index + 1].base.day, previous_nav, direction)
        if sum(table.stream.tell() for table in account.tables.values()) > MAX_REPLAY_BYTES:
            raise ValueError("aggregate portfolio artifacts exceed byte budget")
        check_budget()
        if previous_nav is None:
            raise ValueError("portfolio has no marked session")
        return Ledger(
            {name: table.content() for name, table in account.tables.items()},
            account.order_count,
            account.fill_count,
            previous_nav,
        )
