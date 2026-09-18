"""Independent exact-rational decision and cash-flow arithmetic, not primary kernels."""

import csv
import io
from dataclasses import dataclass
from decimal import Decimal, localcontext
from fractions import Fraction
from typing import Literal

from loop_zipline.models import Policy, Row

F = Fraction


def rounded(
    value: Fraction, scale: int = 10**8, mode: Literal["up", "down", "even"] = "even"
) -> Fraction:
    scaled = value * scale
    if mode == "up":
        units = -(-scaled.numerator // scaled.denominator)
    elif mode == "down":
        units = scaled.numerator // scaled.denominator
    else:
        units = round(scaled)
    return F(units, scale)


def text(value: Fraction | float | int | str | None) -> str:
    if value is None:
        return ""
    if isinstance(value, Fraction):
        with localcontext() as context:
            context.prec = 80
            result = format(Decimal(value.numerator) / Decimal(value.denominator), "f")
        return result.rstrip("0").rstrip(".") if "." in result else result
    return format(value, ".17g") if isinstance(value, float) else str(value)


class Tables:
    def __init__(self, market: bool) -> None:
        self.headers = {
            "targets": "decision_session,execute_session,security_id,shares",
            "orders": "order_id,decision_session,execute_session,security_id,side,shares,"
            "filled_shares,status" + (",reason" if market else ""),
            "fills": "order_id,session,security_id,at_ms,side,shares,price_usd",
            "positions": "session,security_id,shares,mark_usd,market_value_usd",
            "nav": "session,cash_usd,receivable_usd,market_value_usd,gross_value_usd,"
            "short_collateral_usd,nav_usd"
            if market
            else "session,cash_usd,market_value_usd,nav_usd",
            "returns": "session,previous_nav_usd,nav_usd,simple_return",
            "costs": "event_id,session,security_id,kind,cash_delta_usd,receivable_delta_usd,"
            "commission_usd,sec_fee_usd,taf_fee_usd,spread_cost_usd,impact_cost_usd"
            if market
            else "order_id,session,commission_usd,spread_cost_usd,cash_delta_usd",
        }
        self.streams = {name: io.StringIO(newline="") for name in self.headers}
        self.writers = {
            name: csv.writer(stream, lineterminator="\n") for name, stream in self.streams.items()
        }
        for name, header in self.headers.items():
            self.writers[name].writerow(header.split(","))

    def add(self, name: str, *values: Fraction | float | int | str | None) -> None:
        if len(values) != len(self.headers[name].split(",")):
            raise ValueError("independent ledger width differs")
        self.writers[name].writerow([text(value) for value in values])
        if self.streams[name].tell() > 64 * 1024 * 1024:
            raise ValueError("independent ledger byte bound")

    def finish(self) -> dict[str, bytes]:
        result = {name: stream.getvalue().encode("ascii") for name, stream in self.streams.items()}
        if sum(map(len, result.values())) > 64 * 1024 * 1024:
            raise ValueError("independent aggregate byte bound")
        return result


@dataclass(frozen=True)
class Quote:
    price: Fraction
    commission: Fraction
    sec: Fraction
    taf: Fraction
    spread: Fraction
    impact: Fraction

    @property
    def fees(self) -> Fraction:
        return self.commission + self.sec + self.taf


def quote(row: Row, amount: int, policy: Policy, market: bool) -> Quote:
    if not amount or row.opening is None:
        raise ValueError("a fill needs a visible opening and nonzero size")
    count, side = abs(amount), 1 if amount > 0 else -1
    opening = F(row.opening)
    spread = opening * policy.half_spread_bps / 10000
    impact = F(0)
    if market:
        if not row.auction_volume or row.opening_terms is None:
            raise ValueError("a market fill needs auction and trading terms")
        impact = opening * policy.impact_bps * count / (10000 * row.auction_volume)
    price = rounded(opening + side * (spread + impact), mode="up" if side > 0 else "down")
    if price <= 0:
        raise ValueError("modeled price is nonpositive")
    commission = max(F(policy.minimum_commission_usd), count * F(policy.commission_per_share_usd))
    sec = taf = F(0)
    if market and amount < 0 and row.opening_terms is not None:
        terms = row.opening_terms
        sec = rounded(count * price * F(terms.sec_fee_usd_per_million) / 10**6, 100, "up")
        taf = rounded(
            min(count * F(terms.taf_fee_usd_per_share), F(terms.taf_fee_cap_usd)), 100, "up"
        )
    spread_cost = rounded(count * spread) if market else count * abs(price - opening)
    return Quote(
        price, commission, sec, taf, spread_cost, count * abs(price - opening) - spread_cost
    )
