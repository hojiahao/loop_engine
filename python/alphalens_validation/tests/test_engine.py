import ast
import importlib.util
import math
from datetime import date
from pathlib import Path

import pytest

import loop_alphalens
from loop_alphalens.engine import RAW_COLUMNS, calculate, columns, csv_bytes, table
from loop_alphalens.models import Inputs
from loop_alphalens.workflow import compare


def inputs(securities: int = 6) -> Inputs:
    ref = {"sha256": "sha256:" + "a" * 64, "byte_size": 1}
    return Inputs(
        schema="loop.alphalens-input/v1",
        profile="alphalens-statistics.1",
        primary_statistics=ref,
        primary_backtest=ref,
        observations=ref,
        primary_cross_sections=ref,
        source_code_sha256=ref["sha256"],
        environment_sha256=ref["sha256"],
        sessions=(date(2010, 1, 7), date(2010, 1, 8), date(2010, 1, 11), date(2010, 1, 12)),
        securities=tuple(f"US.{index:02}" for index in range(securities)),
        direction="higher_is_better",
        groups=3,
        minimum_cross_section=3,
        minimum_sessions=8,
        production_eligible=False,
    )


def observations(spec: Inputs) -> list[list[object]]:
    rows = []
    for index, day in enumerate(spec.sessions):
        for column, security in enumerate(spec.securities):
            label = spec.sessions[index + 1] if index + 1 < len(spec.sessions) else None
            rows.append(
                [
                    day,
                    security,
                    1,
                    column + 1,
                    label or "",
                    100 if label else "",
                    100 + column if label else "",
                ]
            )
    return rows


def execute(
    spec: Inputs, raw: list[list[object]] | None = None
) -> tuple[list[dict[str, str]], bytes, int]:
    cross, turnover, count = calculate(
        spec, csv_bytes(RAW_COLUMNS, raw or observations(spec)), lambda: None
    )
    return table(cross, columns(spec.groups)), turnover, count


def test_numeric_goldens() -> None:
    rows, _, count = execute(inputs())
    assert count == 3
    assert float(rows[0]["ic"]) == pytest.approx(1)
    assert float(rows[0]["rank_ic"]) == pytest.approx(1)
    assert [float(rows[0][f"group_{index}"]) for index in (1, 2, 3)] == pytest.approx(
        [0.005, 0.025, 0.045]
    )
    assert float(rows[0]["spread"]) == pytest.approx(0.04)
    assert float(rows[0]["monotonicity"]) == pytest.approx(1)
    assert rows[-1]["status"] == "no_forward_session" and rows[-1]["labels"] == "0"


def test_uneven_groups() -> None:
    rows, _, _ = execute(inputs(8))
    assert [float(rows[0][f"group_{index}"]) for index in (1, 2, 3)] == pytest.approx(
        [0.01, 0.04, 0.065]
    )


def test_reversed_direction() -> None:
    rows, _, _ = execute(inputs().model_copy(update={"direction": "lower_is_better"}))
    assert float(rows[0]["ic"]) == pytest.approx(-1)
    assert float(rows[0]["rank_ic"]) == pytest.approx(-1)
    assert float(rows[0]["spread"]) == pytest.approx(-0.04)


def test_tie_order() -> None:
    spec = inputs()
    raw = observations(spec)
    raw[1][3] = raw[0][3]
    rows, _, _ = execute(spec, raw)
    assert float(rows[0]["rank_ic"]) == pytest.approx(0.9856107606091624)
    assert float(rows[0]["group_1"]) == pytest.approx(0.005)


@pytest.mark.parametrize(
    "kind",
    ["constant_signal", "constant_labels", "missing_forward_prices", "insufficient_cross_section"],
)
def test_missing_states(kind: str) -> None:
    spec = inputs()
    raw = observations(spec)
    for column in range(6):
        if kind == "constant_signal":
            raw[column][3] = 1
        elif kind == "constant_labels":
            raw[column][6] = 100
        elif kind == "insufficient_cross_section" and column > 1:
            raw[column][2] = 0
    if kind == "missing_forward_prices":
        raw[0][5] = ""
    rows, _, _ = execute(spec, raw)
    assert rows[0]["status"] == kind
    assert rows[0]["ic"] == "" and rows[0]["rank_ic"] == ""


def test_weekend_turnover() -> None:
    spec = inputs()
    raw = observations(spec)
    for offset in (6, 12):
        for column in range(6):
            raw[offset + column][3] = 6 if column == 0 else column
    _, content, _ = execute(spec, raw)
    rows = table(content, ("session", "quantile", "status", "membership_turnover"))
    assert all(row["status"] == "unavailable" for row in rows[:3])
    assert [float(row["membership_turnover"]) for row in rows[3:6]] == [0.5, 0.5, 0.5]
    assert [float(row["membership_turnover"]) for row in rows[6:]] == [0, 0, 0]


def test_gap_turnover() -> None:
    spec = inputs()
    raw = observations(spec)
    raw[6][5] = ""
    _, content, _ = execute(spec, raw)
    rows = table(content, ("session", "quantile", "status", "membership_turnover"))
    assert all(row["status"] == "unavailable" for row in rows)


@pytest.mark.parametrize(
    "mutation", ["missing", "duplicate", "order", "shift", "infinite", "terminal"]
)
def test_invalid_grid(mutation: str) -> None:
    spec = inputs()
    raw = observations(spec)
    if mutation == "missing":
        raw.pop()
    elif mutation == "duplicate":
        raw[1] = raw[0]
    elif mutation == "order":
        raw.reverse()
    elif mutation == "shift":
        raw[0][4] = "2010-01-11"
    elif mutation == "infinite":
        raw[0][3] = math.inf
    else:
        raw[-1][5] = 100
    with pytest.raises(ValueError):
        execute(spec, raw)


def test_detect_difference() -> None:
    spec = inputs()
    independent, _, _ = calculate(spec, csv_bytes(RAW_COLUMNS, observations(spec)), lambda: None)
    primary = table(independent, columns(spec.groups))
    primary[0]["rank_ic"] = "0"
    mismatches = compare(
        csv_bytes(columns(spec.groups), [list(row.values()) for row in primary]), independent, spec
    )
    assert len(mismatches) == 1
    assert mismatches[0]["field"] == "rank_ic"
    assert mismatches[0]["reason"] == "numerical_difference"


def test_import_boundary() -> None:
    assert importlib.util.find_spec("loop_research") is None
    for path in Path(loop_alphalens.__file__).parent.glob("*.py"):
        for node in ast.walk(ast.parse(path.read_text())):
            if isinstance(node, ast.ImportFrom):
                assert not (node.module or "").startswith("loop_research")
            elif isinstance(node, ast.Import):
                assert all(not name.name.startswith("loop_research") for name in node.names)


def test_malformed_csv() -> None:
    with pytest.raises(ValueError, match="invalid independent CSV"):
        table(b'column\n"unterminated', ("column",))


def test_row_budget() -> None:
    with pytest.raises(ValueError, match="row bounds"):
        table(b"column\n" + b"1\n" * 100_001, ("column",))
