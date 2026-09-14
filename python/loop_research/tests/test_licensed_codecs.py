"""Independent vendor contracts, exact numerical fields and negative normalization."""

import json
from datetime import timedelta
from pathlib import Path
from typing import Any

import pytest
from databento.reference.api.corporate import CorporateActionsHttpAPI
from databento.reference.api.security import SecurityMasterHttpAPI
from licensed_helpers import (
    SEP_COLUMNS,
    SEP_ROW,
    captured,
    jsonl,
    license_config,
    reference_row,
    table_bytes,
)
from nasdaqdatalink.util import Util
from requests import Response

from loop_research.data import databento, sharadar, wrds
from loop_research.data.licensed_config import DatabentoRequest, SharadarRequest, WrdsRequest


def test_sharadar_encoding_matches_official_converter(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path)
    assert isinstance(config, SharadarRequest)
    expected = Util.convert_options(
        "get",
        params={
            "ticker": "DEMO",
            "date": {"gte": "2026-08-01", "lte": "2026-08-31"},
            "qopts": {"columns": ",".join(name for name, _ in SEP_COLUMNS)},
        },
    )["params"]
    assert sharadar.parameters(config, "SEP") == expected


def test_reordered_columns_preserve_prices(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path)
    assert isinstance(config, SharadarRequest)
    table = sharadar.normalize(config, "SEP", [captured(table_bytes())])
    reordered = captured(table_bytes(list(reversed(SEP_COLUMNS)), [list(reversed(SEP_ROW))]))
    assert sharadar.normalize(config, "SEP", [reordered]) == table
    values = dict(zip(table.columns, table.rows[0], strict=True))
    assert (values["close"], values["closeunadj"], values["volume"]) == ("11", "44", "4000")
    assert "raw" not in table.semantics


def test_high_precision_source_decimal_survives(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path)
    assert isinstance(config, SharadarRequest)
    payload = table_bytes().replace(b", 11,", b", 10.123456789012345678,")
    table = sharadar.normalize(config, "SEP", [captured(payload)])
    assert table.rows[0][table.columns.index("close")] == "10.123456789012345678"


@pytest.mark.parametrize(
    "mutation",
    ["missing", "duplicate", "type", "width", "cursor", "out_of_scope", "price", "float_infinity"],
)
def test_sharadar_schema_and_scope_fail_closed(tmp_path: Path, mutation: str) -> None:
    config, _ = license_config(tmp_path)
    assert isinstance(config, SharadarRequest)
    body = json.loads(table_bytes())
    columns = body["datatable"]["columns"]
    rows = body["datatable"]["data"]
    if mutation == "missing":
        columns.pop()
    elif mutation == "duplicate":
        columns[-1] = columns[0]
    elif mutation == "type":
        columns[2]["type"] = "String"
    elif mutation == "width":
        rows[0].pop()
    elif mutation == "cursor":
        del body["meta"]["next_cursor_id"]
    elif mutation == "out_of_scope":
        rows[0][1] = "2026-09-01"
    elif mutation == "price":
        rows[0][2] = 999
    else:
        rows[0][2] = float("inf")
    with pytest.raises(ValueError):
        sharadar.normalize(config, "SEP", [captured(json.dumps(body).encode())])


def test_same_daily_key_cannot_be_ingested_twice(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path)
    assert isinstance(config, SharadarRequest)
    with pytest.raises(ValueError, match="duplicate native key"):
        sharadar.normalize(config, "SEP", [captured(table_bytes()), captured(table_bytes())])


def test_as_reported_dimension_retains_three_source_dates(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path, tables=["SF1"])
    assert isinstance(config, SharadarRequest)
    columns = [
        ("ticker", "String"),
        ("dimension", "String"),
        ("calendardate", "Date"),
        ("datekey", "Date"),
        ("reportperiod", "Date"),
        ("lastupdated", "Date"),
        ("assets", "BigDecimal"),
        ("liabilities", "BigDecimal"),
        ("equity", "BigDecimal"),
        ("revenue", "BigDecimal"),
        ("netinc", "BigDecimal"),
        ("sharesbas", "BigDecimal"),
    ]
    row = [
        "DEMO",
        "ARQ",
        "2026-06-30",
        "2026-08-03",
        "2026-06-27",
        "2026-08-05",
        1000,
        400,
        600,
        30,
        None,
        100,
    ]
    table = sharadar.normalize(config, "SF1", [captured(table_bytes(columns, [row]))])
    values = dict(zip(table.columns, table.rows[0], strict=True))
    assert values["netinc"] is None
    assert values["datekey"] == "2026-08-03" and values["reportperiod"] == "2026-06-27"
    assert "datekey_not_verified_known_at" in table.semantics
    row[1] = "MRQ"
    with pytest.raises(ValueError, match="as-reported"):
        sharadar.normalize(config, "SF1", [captured(table_bytes(columns, [row]))])


def test_corporate_actions_keep_null_contra_and_distinct_rows(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path, tables=["ACTIONS"])
    assert isinstance(config, SharadarRequest)
    columns = [
        ("date", "Date"),
        ("action", "String"),
        ("ticker", "String"),
        ("name", "String"),
        ("value", "BigDecimal"),
        ("contraticker", "String"),
        ("contraname", "String"),
    ]
    row = ["2026-08-05", "dividend", "DEMO", "Invented company", 1, None, None]
    table = sharadar.normalize(config, "ACTIONS", [captured(table_bytes(columns, [row]))])
    assert table.rows[0][-2:] == (None, None)
    assert len(table.rows[0][0]) == 64


@pytest.mark.parametrize("dataset", ["security_master", "corporate_actions"])
def test_databento_form_matches_native_sdk(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, dataset: str
) -> None:
    config, _ = license_config(tmp_path, "databento", datasets=[dataset])
    assert isinstance(config, DatabentoRequest)
    api_type = SecurityMasterHttpAPI if dataset == "security_master" else CorporateActionsHttpAPI
    recorded: dict[str, Any] = {}

    def intercept(self: object, **kwargs: Any) -> Response:
        recorded.update(kwargs)
        raise RuntimeError("contract captured before network")

    monkeypatch.setattr(api_type, "_post", intercept)
    api = api_type("unused-synthetic-key", "https://hist.databento.com")
    options = (
        {"index": "ts_effective"}
        if dataset == "security_master"
        else {"index": "event_date", "pit": True, "flatten": False}
    )
    with pytest.raises(RuntimeError, match="contract captured"):
        api.get_range(
            start=config.start,
            end=config.end + timedelta(days=1),
            symbols=list(config.listing_ids),
            stype_in="listing_id",
            countries=["US"],
            allocate_isins=False,
            **options,
        )
    assert recorded["url"] == databento.endpoint(dataset)
    assert recorded["basic_auth"] is True
    expected = {
        key: str(value).lower() if isinstance(value, bool) else str(value)
        for key, value in recorded["data"].items()
        if value is not None
    }
    # The SDK always requests zstd; bounded raw capture explicitly negotiates none.
    expected["compression"] = "none"
    assert databento.parameters(config, dataset) == expected


def test_databento_retains_cancellation_and_nanoseconds(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path, "databento", datasets=["corporate_actions"])
    assert isinstance(config, DatabentoRequest)
    row = reference_row(
        event_unique_id="synthetic-event",
        event_date="2026-08-20",
        event="DIV",
        event_action="CANCEL",
        rate_info={"declared_gross_amount": "0.123456789012345678"},
    )
    table = databento.normalize(config, "corporate_actions", captured(jsonl(row)))
    values = dict(zip(table.columns, table.rows[0], strict=True))
    assert values["event_action"] == "CANCEL"
    assert values["ts_record"] == "2026-08-29T12:00:00.123456789Z"
    assert values["rate_info.declared_gross_amount"] == "0.123456789012345678"


@pytest.mark.parametrize(
    "change",
    [
        {"listing_id": "L-1"},
        {"security_id": "I-999999"},
        {"issuer_id": None},
        {"listing_country": "GB"},
        {"ts_effective": "2026-09-01T00:00:00Z"},
        {"ts_created": "2026-09-13T12:00:00.000000001Z"},
        {"ts_record": "2026-08-01"},
        {"unexpected": [1, 2, 3]},
    ],
)
def test_databento_rejects_wrong_identity_or_clocks(tmp_path: Path, change: dict[str, Any]) -> None:
    config, _ = license_config(tmp_path, "databento")
    assert isinstance(config, DatabentoRequest)
    with pytest.raises(ValueError):
        databento.normalize(config, "security_master", captured(jsonl(reference_row(**change))))


def test_equal_timestamp_encodings_cannot_duplicate_a_vintage(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path, "databento")
    assert isinstance(config, DatabentoRequest)
    with pytest.raises(ValueError, match="duplicate native key"):
        databento.normalize(
            config,
            "security_master",
            captured(
                jsonl(reference_row(), reference_row(ts_effective="2026-08-28T00:00:00.000000000Z"))
            ),
        )


def test_crsp_ciz_does_not_apply_delisting_twice(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path, "wrds")
    assert isinstance(config, WrdsRequest)
    content = json.dumps(
        {
            "format": "postgres_text_projection/v1",
            "columns": [
                "permno",
                "dlycaldt",
                "dlyprc",
                "dlyprcflg",
                "dlyvol",
                "dlyret",
                "dlyretx",
                "dlydelflg",
            ],
            "rows": [["999999", "2026-08-28", "0.052", "DP", None, "-0.6", "-0.6", "Y"]],
        }
    ).encode()
    table = wrds.normalize(config, captured(content))
    assert table.rows[0][5] == "-0.6"
    assert "crsp_ciz_includes_delisting" in table.semantics
    sql, params = wrds.query(config)
    assert "dlret" not in sql and "crsp.stkdlysecuritydata" in sql
    assert params[0] == [999999] and params[-1] == 10001


def test_compustat_query_fixes_reporting_format(tmp_path: Path) -> None:
    config, _ = license_config(
        tmp_path, "wrds", profile="compustat_fundq_v1", identifiers=["123456"]
    )
    assert isinstance(config, WrdsRequest)
    sql, params = wrds.query(config)
    assert "comp.fundq" in sql and "datafmt = 'STD'" in sql and "consol = 'C'" in sql
    assert params[0] == ["123456"]
    assert "123456" not in sql


def test_current_permaticker_is_separate_from_historical_status(tmp_path: Path) -> None:
    config, _ = license_config(tmp_path, tables=["TICKERS"])
    assert isinstance(config, SharadarRequest)
    columns = [
        ("table", "String"),
        ("permaticker", "Integer"),
        ("ticker", "String"),
        ("name", "String"),
        ("exchange", "String"),
        ("isdelisted", "String"),
        ("category", "String"),
        ("currency", "String"),
        ("firstpricedate", "Date"),
        ("lastpricedate", "Date"),
        ("firstadded", "Date"),
        ("lastupdated", "Date"),
        ("relatedtickers", "String"),
        ("siccode", "Integer"),
        ("sicsector", "String"),
        ("sicindustry", "String"),
        ("famaindustry", "String"),
        ("sector", "String"),
        ("industry", "String"),
        ("location", "String"),
        ("scalemarketcap", "String"),
        ("scalerevenue", "String"),
    ]
    row = [
        "SEP",
        999999,
        "DEMO",
        "Invented company",
        "NASDAQ",
        "Y",
        "Domestic Common Stock",
        "USD",
        "2005-01-03",
        "2026-08-28",
        "2015-01-01",
        "2026-08-31",
        None,
        1234,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ]
    table = sharadar.normalize(config, "TICKERS", [captured(table_bytes(columns, [row]))])
    values = dict(zip(table.columns, table.rows[0], strict=True))
    assert values["permaticker"] == "999999" and values["isdelisted"] == "Y"
    assert "not_historical_universe" in table.semantics
    assert "issuer_id" not in table.columns
    columns[1] = ("permaticker", "String")
    columns[13] = ("siccode", "String")
    row[1], row[13] = "DEMO-001", "0011"
    textual = sharadar.normalize(config, "TICKERS", [captured(table_bytes(columns, [row]))])
    values = dict(zip(textual.columns, textual.rows[0], strict=True))
    assert values["permaticker"] == "DEMO-001" and values["siccode"] == "0011"
