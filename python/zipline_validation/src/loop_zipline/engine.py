"""Event-driven independent strategy, constraints, actions and Zipline accounting.

Exact rational arithmetic determines whole shares and dated cash obligations.
Zipline independently maintains actual fills, positions, transaction cash and
native valuation. No primary target, order, cost or NAV enters this calculation.
"""

import heapq
from collections.abc import Callable
from datetime import date
from fractions import Fraction as F
from itertools import groupby

from loop_zipline.accounting import Tables, quote, rounded, text
from loop_zipline.artifacts import encode
from loop_zipline.models import Event, Inputs, Row, Session, Tape, Terms
from loop_zipline.native import Broker, Unavailable


class Replay:
    def __init__(self, inputs: Inputs, tape: Tape, check: Callable[[], None]) -> None:
        self.policy, self.tape, self.check = inputs.policy, tape, check
        self.market = inputs.engine == "pit-actions-long-short.1"
        self.direction = -1 if inputs.direction == "higher_is_better" else 1
        self.ids = tuple(row.security_id for row in tape.sessions[0].rows)
        self.cash = F(self.policy.initial_cash_usd)
        self.broker = Broker([session.day for session in tape.sessions], self.ids, self.cash)
        self.marks = dict.fromkeys(self.ids, F(0))
        self.tables = Tables(self.market)
        self.pending: dict[str, int] = {}
        self.order_ids: dict[str, str] = {}
        self.claims: list[tuple[int, str, str, F]] = []
        self.retired: set[str] = set()
        self.prior_terms: dict[str, Terms] = {}
        self.order_count = 0
        self.bridge: list[dict[str, object]] = []
        if not self.market and any(
            session.actions
            or any(
                row.auction_volume is not None
                or row.opening_terms is not None
                or row.closing_terms is not None
                for row in session.rows
            )
            for session in tape.sessions
        ):
            raise ValueError("version-1 profile cannot consume market actions or terms")
        if self.market and any(
            row.auction_volume is None for session in tape.sessions for row in session.rows
        ):
            raise ValueError("market profile needs explicit auction volumes")

    def identity(self) -> str:
        self.order_count += 1
        if self.order_count > 200_000:
            raise ValueError("independent order bound")
        return f"order.{self.order_count:08d}"

    def exposure(self) -> tuple[F, F, F, F]:
        positions = [self.broker.shares(identity) * self.marks[identity] for identity in self.ids]
        claims = sum((claim[3] for claim in self.claims), F(0))
        net, gross = sum(positions, F(0)), sum(map(abs, positions), F(0))
        return self.cash + claims + net, net, gross, claims

    def collateral(self, identity: str) -> F:
        per_share = rounded(
            self.marks[identity] * self.policy.short_collateral_bps / 10000, 1, "up"
        )
        return max(0, -self.broker.shares(identity)) * per_share

    def cost(
        self,
        day: date,
        identity: str,
        security: str,
        kind: str,
        cash: F = F(0),
        claims: F = F(0),
        commission: F = F(0),
        sec: F = F(0),
        taf: F = F(0),
        spread: F = F(0),
        impact: F = F(0),
        *,
        native: bool = True,
    ) -> None:
        self.cash += cash
        if native and cash:
            self.broker.flow(cash)
        self.tables.add(
            "costs",
            identity,
            day.isoformat(),
            security,
            kind,
            cash,
            claims,
            commission,
            sec,
            taf,
            spread,
            impact,
        )

    def finance(self, before: date, after: date) -> None:
        interval = (after - before).days
        balance = self.cash - sum((self.collateral(identity) for identity in self.ids), F(0))
        balance += sum((item[3] for item in self.claims if item[3] < 0), F(0))
        rate = self.policy.cash_credit_bps if balance >= 0 else self.policy.cash_debit_bps
        interest = rounded(balance * rate * interval / (10000 * self.policy.day_count), mode="down")
        if interest:
            self.cost(after, "finance." + before.isoformat(), "", "cash_interest", interest)
        for identity in self.ids:
            if self.broker.shares(identity) < 0:
                terms = self.prior_terms.get(identity)
                if terms is None:
                    raise ValueError("missing prior short borrow terms")
                amount = rounded(
                    self.collateral(identity)
                    * terms.borrow_rate_bps
                    * interval
                    / (10000 * self.policy.day_count),
                    mode="up",
                )
                self.cost(after, "borrow." + before.isoformat(), identity, "borrow_fee", -amount)

    def pay(self, at: int, day: date) -> None:
        while self.claims and self.claims[0][0] <= at:
            self.check()
            _, identity, security, amount = heapq.heappop(self.claims)
            self.cost(day, identity, security, "payment", amount, -amount)

    def claim(self, event: Event, day: date, amount: F, kind: str) -> None:
        if not amount:
            return
        if event.pay_ms is None:
            raise ValueError("unresolved entitlement payment")
        heapq.heappush(self.claims, (event.pay_ms, event.event_id, event.security_id, amount))
        self.cost(day, event.event_id, event.security_id, kind, claims=amount)

    def action(self, event: Event, day: date, previous: date | None) -> None:
        security = event.security_id
        if security in self.retired:
            raise ValueError("action follows cash delisting")
        shares = self.broker.shares(security)
        if event.kind == "split":
            if event.numerator is None or event.denominator is None:
                raise ValueError("missing split ratio")
            ratio = F(event.numerator, event.denominator)
            converted = shares * ratio
            whole = int(converted)
            if abs(whole) > 10**12:
                raise ValueError("split quantity bound")
            remainder = converted - whole
            if remainder:
                if event.fraction_price_usd is None:
                    raise ValueError("fractional split needs explicit settlement terms")
                self.claim(
                    event, day, rounded(remainder * F(event.fraction_price_usd)), "cash_in_lieu"
                )
            self.marks[security] /= ratio
            # Native split logic pays cost-basis cash immediately and floors
            # negative shares. This explicit pinned position update implements
            # the frozen economic entitlement instead; every flow is retained.
            self.broker.mark(security, self.marks[security], event.at_ms, shares=whole)
            if security in self.pending:
                self.pending[security] = int(self.pending[security] * ratio)
                if abs(self.pending[security]) > 10**12:
                    raise ValueError("split target bound")
            self.cost(day, event.event_id, security, "split")
        else:
            if event.amount_per_share_usd is None:
                raise ValueError("missing cash entitlement amount")
            self.claim(event, day, rounded(shares * F(event.amount_per_share_usd)), event.kind)
            if event.kind == "delisting":
                if security in self.pending and previous is not None:
                    requested = self.pending.pop(security) - shares
                    self.order(
                        day, previous, security, requested, 0, "delisted", "signal", self.identity()
                    )
                self.broker.mark(security, F(0), event.at_ms, shares=0)
                self.marks[security] = F(0)
                self.retired.add(security)
                self.cost(day, event.event_id, security, "retired")

    def order(
        self,
        day: date,
        decision: date,
        security: str,
        amount: int,
        filled: int,
        status: str,
        reason: str,
        identity: str,
    ) -> None:
        common = (
            identity,
            decision.isoformat(),
            day.isoformat(),
            security,
            "buy" if amount > 0 else "sell",
            abs(amount),
            filled,
            status,
        )
        self.tables.add("orders", *common, *((reason,) if self.market else ()))

    def margin(self, row: Row, requested: int) -> int:
        count, sign = abs(requested), 1 if requested > 0 else -1
        held = self.broker.shares(row.security_id)
        reduction = min(count, abs(held)) if held * sign < 0 else 0
        nav, _, gross, _ = self.exposure()
        collateral_nav = nav - sum((claim[3] for claim in self.claims if claim[3] > 0), F(0))
        mark = self.marks[row.security_id]

        def feasible(quantity: int) -> bool:
            if not quantity:
                return True
            result = quote(row, sign * quantity, self.policy, True)
            equity = collateral_nav + sign * quantity * (mark - result.price) - result.fees
            exposure = gross + (abs(held + sign * quantity) - abs(held)) * mark
            return equity > 0 and equity * 10000 >= exposure * self.policy.initial_margin_bps

        if reduction and not feasible(reduction):
            return reduction
        low, high = reduction, count
        while low < high:
            self.check()
            trial = (low + high + 1) // 2
            if feasible(trial):
                low = trial
            else:
                high = trial - 1
        return low

    def quantity(self, row: Row, requested: int) -> tuple[int, str]:
        count = abs(requested)
        if row.opening is None:
            return 0, "missing_open"
        if not self.market:
            pricing = quote(row, requested, self.policy, False)
            if requested > 0:
                budget = min(
                    (self.cash - F(self.policy.minimum_commission_usd)) / pricing.price,
                    self.cash / (pricing.price + F(self.policy.commission_per_share_usd)),
                )
                count = min(count, max(0, budget // self.policy.lot_size) * self.policy.lot_size)
            elif self.cash + count * pricing.price < pricing.fees:
                count = 0
            return count, "filled" if count == abs(
                requested
            ) else "partial_cash" if count else "cash"
        terms = row.opening_terms
        if terms is None:
            return 0, "missing_terms"
        if not terms.tradable:
            return 0, "untradable"
        capacity = (row.auction_volume or 0) * self.policy.participation_bps // 10000
        status = "filled"
        if count > capacity:
            count, status = capacity, "capacity"
        if requested < 0:
            held = self.broker.shares(row.security_id)
            allowed = 0 if terms.recalled else terms.borrow_limit
            limit = allowed if terms.short_allowed else min(allowed, max(0, -held))
            if count > max(0, held + limit):
                count, status = max(0, held + limit), "borrow"
        if count:
            affordable = self.margin(row, count if requested > 0 else -count)
            if affordable < count:
                count, status = affordable, "margin"
        return count, "partial_" + status if 0 < count < abs(requested) else status

    def opening(self, session: Session, row: Row, previous: date | None) -> None:
        security, terms = row.security_id, row.opening_terms
        held = self.broker.shares(security)
        if security in self.retired:
            if row.opening is not None or row.eligible:
                raise ValueError("cash-delisted security resumed trading")
            return
        if self.market and held < 0 and terms is None:
            raise ValueError("short lacks opening borrow terms")
        target = self.pending.pop(security, held)
        allowed = 0 if terms is None or terms.recalled else terms.borrow_limit
        recall = self.market and held < -allowed
        if recall:
            target = max(target, -allowed)
        requested = target - held
        if not requested:
            return
        count, status = self.quantity(row, requested)
        identity = self.identity() if self.market else self.order_ids.pop(security)
        result = (
            quote(row, count if requested > 0 else -count, self.policy, self.market)
            if count
            else None
        )
        actual, price = self.broker.execute(
            security,
            identity,
            row.open_ms or session.scheduled_open_ms,
            requested,
            count,
            result.price if result else F(0),
            result.fees if result else F(0),
        )
        if actual != count or self.broker.shares(security) != held + (
            count if requested > 0 else -count
        ):
            raise ValueError("Zipline fill/position differs from independent execution")
        self.order(
            session.day,
            session.day if recall else previous or session.day,
            security,
            requested,
            actual,
            status,
            "borrow_recall" if recall else "signal",
            identity,
        )
        if result is not None:
            signed = count if requested > 0 else -count
            delta = -signed * result.price - result.fees
            self.tables.add(
                "fills",
                identity,
                session.day.isoformat(),
                security,
                row.open_ms,
                "buy" if signed > 0 else "sell",
                count,
                price,
            )
            if self.market:
                self.cost(
                    session.day,
                    identity,
                    security,
                    "trade",
                    delta,
                    commission=result.commission,
                    sec=result.sec,
                    taf=result.taf,
                    spread=result.spread,
                    impact=result.impact,
                    native=False,
                )
            else:
                self.cash += delta
                self.tables.add(
                    "costs",
                    identity,
                    session.day.isoformat(),
                    result.commission,
                    result.spread,
                    delta,
                )
            if self.exposure()[0] <= 0:
                raise ValueError("insolvency after execution")
        if recall and self.broker.shares(security) < -allowed:
            raise ValueError("unmet borrow recall")

    def close(self, session: Session, prior: float | None) -> tuple[F, float]:
        self.prior_terms = {}
        for row in session.rows:
            shares = self.broker.shares(row.security_id)
            if shares and row.closing is None:
                raise ValueError("held security lacks closing mark")
            mark = F(row.closing or "0")
            self.marks[row.security_id] = mark
            self.broker.mark(row.security_id, mark, session.decision_ms)
            if self.market and shares < 0:
                if (
                    row.closing_terms is None
                    or row.closing_terms.recalled
                    or row.closing_terms.borrow_limit < -shares
                ):
                    raise ValueError("short lacks closing availability")
                self.prior_terms[row.security_id] = row.closing_terms
            actual, native_mark = self.broker.position(row.security_id)
            self.tables.add(
                "positions",
                session.day.isoformat(),
                row.security_id,
                actual,
                native_mark if row.closing is not None else None,
                actual * native_mark,
            )
        exact, _, gross, claims = self.exposure()
        if not 0 < exact <= 10**24 or exact * 10000 < gross * self.policy.maintenance_margin_bps:
            raise ValueError("closing solvency or maintenance margin")
        cash, net, native = self.broker.values()
        if abs(F(cash) - self.cash) > F(1, 100000):
            raise Unavailable("zipline_accumulated_cash_precision")
        adjusted = native + float(claims)
        day = session.day.isoformat()
        collateral = sum((self.collateral(identity) for identity in self.ids), F(0))
        if self.market:
            self.tables.add("nav", day, cash, claims, net, gross, collateral, adjusted)
        else:
            self.tables.add("nav", day, cash, net, native)
        self.tables.add(
            "returns", day, prior, adjusted, None if prior is None else adjusted / prior - 1
        )
        self.bridge.append(
            {
                "session": day,
                "zipline_cash_usd": text(cash),
                "zipline_nav_usd": text(native),
                "unpaid_claims_usd": text(claims),
                "economic_nav_usd": text(adjusted),
            }
        )
        return exact, adjusted

    def plan(self, session: Session, following: date, nav: F) -> None:
        ranked = sorted(
            (
                row
                for row in session.rows
                if row.eligible and row.factor is not None and row.security_id not in self.retired
            ),
            key=lambda row: (self.direction * (row.factor or 0.0), row.security_id),
        )
        policy = self.policy
        enough = (
            not (policy.long_weight_bps and policy.short_weight_bps)
            or len(ranked) >= 2 * policy.holdings
        )
        long = ranked[: policy.holdings] if policy.long_weight_bps and enough else []
        short = ranked[-policy.holdings :] if policy.short_weight_bps and enough else []
        desired = dict.fromkeys(self.ids, 0)
        for selected, weight, sign in (
            (long, policy.long_weight_bps, 1),
            (short, policy.short_weight_bps, -1),
        ):
            for row in selected:
                if row.closing is None:
                    raise ValueError("selection lacks close for sizing")
                lots = nav * weight // (10000 * len(selected) * F(row.closing) * policy.lot_size)
                quantity = lots * policy.lot_size
                if quantity > 10**12:
                    raise ValueError("target quantity bound")
                desired[row.security_id] = sign * quantity
        self.pending = {}
        for identity, target in desired.items():
            self.tables.add(
                "targets", session.day.isoformat(), following.isoformat(), identity, target
            )
            if target != self.broker.shares(identity):
                self.pending[identity] = target
                if not self.market:
                    self.order_ids[identity] = self.identity()

    def priority(self, row: Row) -> tuple[bool, str]:
        held = self.broker.shares(row.security_id)
        terms = row.opening_terms
        recall = (
            self.market
            and held < 0
            and terms is not None
            and (terms.recalled or terms.borrow_limit < -held)
        )
        return recall or self.pending.get(row.security_id, held) > held, row.security_id

    def run(self) -> tuple[dict[str, bytes], bytes]:
        prior = None
        for index, session in enumerate(self.tape.sessions):
            self.check()
            self.broker.start(session.day)
            previous = self.tape.sessions[index - 1].day if index else None
            if self.market and previous is not None:
                self.finance(previous, session.day)
            events: list[tuple[int, int, str, Event | Row]] = [
                (event.at_ms, 0, event.security_id, event) for event in session.actions
            ]
            events.extend(
                (row.open_ms or session.scheduled_open_ms, 1, row.security_id, row)
                for row in session.rows
            )
            for at, group in groupby(
                sorted(events, key=lambda item: item[:3]), key=lambda item: item[0]
            ):
                self.check()
                self.pay(at, session.day)
                opening = []
                for _, _, identity, event in group:
                    if isinstance(event, Event):
                        self.action(event, session.day, previous)
                    else:
                        if event.opening is not None:
                            self.marks[identity] = F(event.opening)
                            self.broker.mark(identity, self.marks[identity], at)
                        opening.append(event)
                self.pay(at, session.day)
                if self.market and self.exposure()[0] <= 0:
                    raise ValueError("insolvency at observed event")
                for row in sorted(opening, key=self.priority):
                    self.check()
                    self.opening(session, row, previous)
            self.pay(session.decision_ms, session.day)
            exact, prior = self.close(session, prior)
            if index + 1 < len(self.tape.sessions):
                self.plan(session, self.tape.sessions[index + 1].day, exact)
        self.check()
        return self.tables.finish(), encode(
            {"schema": "loop.zipline-bridge/v1", "sessions": self.bridge}
        )


def calculate(
    inputs: Inputs, content: bytes, check: Callable[[], None]
) -> tuple[dict[str, bytes], bytes]:
    tape = Tape.model_validate_json(content)
    return Replay(inputs, tape, check).run()
