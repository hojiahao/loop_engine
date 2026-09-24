"""Pinned Zipline finance adapter, with explicit internal-flow/action extensions.

The event clock supplies observed openings directly to SimulationBlotter. Daily
TradingAlgorithm bars would execute at a different price/time. No primary ledger
is passed to this module. Private ledger hooks are version-pinned and tested.
"""

import math
from collections.abc import Sequence
from datetime import date
from fractions import Fraction
from typing import Any

import pandas as pd  # type: ignore[import-untyped]
from zipline.assets import Equity, ExchangeInfo  # type: ignore[import-untyped]
from zipline.finance.blotter.simulation_blotter import (  # type: ignore[import-untyped]
    SimulationBlotter,
)
from zipline.finance.commission import CommissionModel  # type: ignore[import-untyped]
from zipline.finance.execution import MarketOrder  # type: ignore[import-untyped]
from zipline.finance.ledger import Ledger  # type: ignore[import-untyped]
from zipline.finance.slippage import SlippageModel  # type: ignore[import-untyped]


class Unavailable(ValueError):
    """A valid input exceeds the reviewed independent numerical profile."""


class Opening(SlippageModel):  # type: ignore[misc]
    """Execute the independently risk-limited quantity at its computed raw price."""

    def process_order(self, data: Any, order: Any) -> tuple[float, int]:
        return data.price, data.quantity if order.amount > 0 else -data.quantity


class Fees(CommissionModel):  # type: ignore[misc]
    def __init__(self) -> None:
        self.amount = 0.0

    def calculate(self, order: Any, transaction: Any) -> float:
        return self.amount


class Bar:
    def __init__(self, at: int, quantity: int, price: float) -> None:
        self.current_dt = pd.Timestamp(at, unit="ms", tz="UTC")
        self.quantity, self.price = quantity, price

    def current(self, asset: Any, field: str) -> float:
        if field == "volume":
            return float(self.quantity)
        if field == "close":
            return self.price
        raise ValueError("unsupported independent event field")


class Broker:
    """Zipline owns filled positions, transaction cash, commissions and native NAV."""

    def __init__(self, days: Sequence[date], ids: Sequence[str], capital: Fraction) -> None:
        axis = pd.DatetimeIndex(days, tz="UTC")
        exchange = ExchangeInfo("XNYS", "XNYS", "US")
        self.assets = {
            identity: Equity(index, symbol=identity, exchange_info=exchange)
            for index, identity in enumerate(ids, 1)
        }
        self.ledger = Ledger(axis, float(capital), "daily")
        self.fees = Fees()
        self.blotter = SimulationBlotter(equity_slippage=Opening(), equity_commission=self.fees)
        self.blotter.max_shares = 10**12

    def shares(self, identity: str) -> int:
        position = self.ledger.position_tracker.positions.get(self.assets[identity])
        value = 0 if position is None else position.amount
        if not math.isfinite(value) or value != int(value) or abs(value) > 10**12:
            raise Unavailable("zipline_whole_share_precision")
        return int(value)

    def mark(self, identity: str, price: Fraction, at: int, *, shares: int | None = None) -> None:
        self.ledger.position_tracker.update_position(
            self.assets[identity],
            amount=shares,
            last_sale_price=float(price),
            last_sale_date=pd.Timestamp(at, unit="ms", tz="UTC"),
        )
        self.ledger._dirty_portfolio = True

    def flow(self, amount: Fraction) -> None:
        # Economic internal cash: capital_change would incorrectly classify it
        # as an external deposit. Splits/claims are documented outside this hook.
        self.ledger._cash_flow(float(amount))

    def position(self, identity: str) -> tuple[int, float]:
        position = self.ledger.position_tracker.positions.get(self.assets[identity])
        shares = self.shares(identity)
        mark = 0.0 if position is None else float(position.last_sale_price)
        if not math.isfinite(mark) or math.ulp(mark) > 1e-9 or math.ulp(shares * mark) > 0.000001:
            raise Unavailable("zipline_position_precision")
        return shares, mark

    def start(self, day: date) -> None:
        self.ledger.start_of_session(pd.Timestamp(day, tz="UTC"))

    def execute(
        self,
        identity: str,
        order_id: str,
        at: int,
        requested: int,
        quantity: int,
        price: Fraction,
        fees: Fraction,
    ) -> tuple[int, float | None]:
        if math.ulp(float(price)) > 1e-9:
            raise Unavailable("zipline_price_precision")
        bar = Bar(at, quantity, float(price))
        self.blotter.current_dt = bar.current_dt
        asset = self.assets[identity]
        self.blotter.order(asset, requested, MarketOrder(), order_id=order_id)
        self.fees.amount = float(fees)
        transactions, commissions, closed = self.blotter.get_transactions(bar)
        if len(transactions) != int(quantity > 0):
            raise ValueError("Zipline emitted an unexpected transaction count")
        for transaction in transactions:
            self.ledger.process_transaction(transaction)
        for commission in commissions:
            self.ledger.process_commission(commission)
        order = self.blotter.orders[order_id]
        filled = int(abs(order.filled))
        self.blotter.cancel(order_id)
        self.blotter.prune_orders(closed)
        self.ledger.process_order(order)
        return filled, float(transactions[0].price) if transactions else None

    def values(self) -> tuple[float, float, float]:
        portfolio = self.ledger.portfolio
        result = (
            float(portfolio.cash),
            float(portfolio.positions_value),
            float(portfolio.portfolio_value),
        )
        if not all(math.isfinite(value) for value in result):
            raise Unavailable("zipline_nonfinite_valuation")
        if any(math.ulp(value) > 0.000001 for value in result):
            raise Unavailable("zipline_dollar_precision")
        return result
