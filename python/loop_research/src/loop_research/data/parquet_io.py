"""Deterministic bounded source Parquet, without filesystem or remote discovery."""

from datetime import date
from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]
import pyarrow.parquet as pq  # type: ignore[import-untyped]

MAX_PARQUET_BYTES = 64 * 1024 * 1024
MAX_TABLE_BYTES = 48 * 1024 * 1024
ENVELOPE = (
    "_loop_observation_date",
    "_loop_known_at_ns",
    "_loop_ingested_at_ns",
    "_loop_availability",
)
type Row = tuple[date, int, int, str, tuple[str | None, ...]]


def _schema(columns: tuple[str, ...]) -> Any:
    if not 1 <= len(columns) <= 200 or len(set(columns)) != len(columns):
        raise ValueError("source table column budget or uniqueness failure")
    if any(column.startswith("_loop_") for column in columns):
        raise ValueError("source column conflicts with snapshot envelope")
    return pa.schema(
        [
            pa.field(ENVELOPE[0], pa.date32(), nullable=False),
            pa.field(ENVELOPE[1], pa.int64(), nullable=False),
            pa.field(ENVELOPE[2], pa.int64(), nullable=False),
            pa.field(ENVELOPE[3], pa.string(), nullable=False),
            *(pa.field(column, pa.string(), nullable=True) for column in columns),
        ],
        metadata={b"loop.schema": b"loop.source-table/v1"},
    )


def _table(columns: tuple[str, ...], rows: tuple[Row, ...]) -> Any:
    if len(rows) > 10_000:
        raise ValueError("source Parquet row budget")
    schema = _schema(columns)
    # Account for offsets/nulls as well as scalar bytes before allocating Arrow arrays.
    byte_count = 0
    for business, known, ingested, basis, values in rows:
        if (
            type(business) is not date
            or type(known) is not int
            or type(ingested) is not int
            or known > ingested
            or len(values) != len(columns)
            or len(basis) > 64
        ):
            raise ValueError("invalid source row envelope")
        if any(value is not None and len(value) > 4096 for value in values):
            raise ValueError("source scalar exceeds the byte budget")
        byte_count += 64 + 8 * len(values) + sum(len(value.encode()) for value in values if value)
        if byte_count > MAX_TABLE_BYTES:
            raise ValueError("source table exceeds the uncompressed byte budget")
    arrays = [
        pa.array([row[0] for row in rows], type=pa.date32()),
        pa.array([row[1] for row in rows], type=pa.int64()),
        pa.array([row[2] for row in rows], type=pa.int64()),
        pa.array([row[3] for row in rows], type=pa.string()),
        *(
            pa.array([row[4][index] for row in rows], type=pa.string())
            for index in range(len(columns))
        ),
    ]
    return pa.Table.from_arrays(arrays, schema=schema)


def encode_parquet(columns: tuple[str, ...], rows: tuple[Row, ...]) -> bytes:
    """Use a fixed writer profile; exact textual source scalars survive unchanged."""
    table = _table(columns, rows)
    sink = pa.BufferOutputStream()
    pq.write_table(
        table,
        sink,
        version="2.6",
        compression="zstd",
        compression_level=3,
        use_dictionary=False,
        write_statistics=True,
        row_group_size=2048,
        data_page_version="2.0",
        data_page_size=64 * 1024,
        write_batch_size=256,
        write_page_checksum=True,
        store_schema=True,
    )
    content: bytes = sink.getvalue().to_pybytes()
    if len(content) > MAX_PARQUET_BYTES:
        raise ValueError("source Parquet exceeds the byte budget")
    return content


def verify_parquet(content: bytes, columns: tuple[str, ...], rows: tuple[Row, ...]) -> None:
    """Compare actual schema and values after bounded footer checks.

    The caller verifies the content hash first. Only a local byte buffer is
    parsed; directory discovery, URI resolution and external column files are
    forbidden. Extension types and background reads are disabled. Invalid,
    oversized, corrupt or semantically different data raises ValueError.
    """
    if not 8 <= len(content) <= MAX_PARQUET_BYTES or content[:4] != b"PAR1":
        raise ValueError("invalid bounded Parquet input")
    if content[-4:] != b"PAR1" or int.from_bytes(content[-8:-4], "little") > 2 * 1024 * 1024:
        raise ValueError("source Parquet footer budget")
    expected = _table(columns, rows)
    with pq.ParquetFile(
        pa.BufferReader(content),
        pre_buffer=False,
        arrow_extensions_enabled=False,
        page_checksum_verification=True,
        thrift_string_size_limit=1024 * 1024,
        thrift_container_size_limit=100_000,
    ) as source:
        metadata = source.metadata
        if (
            metadata.num_rows != len(rows)
            or metadata.num_columns != len(columns) + 4
            or metadata.num_row_groups > 8
            or not source.schema_arrow.equals(expected.schema, check_metadata=True)
        ):
            raise ValueError("source Parquet schema or row count mismatch")
        decoded = 0
        for index in range(metadata.num_row_groups):
            group = metadata.row_group(index)
            decoded += group.total_byte_size
            if decoded > MAX_PARQUET_BYTES:
                raise ValueError("source Parquet decoded-byte budget")
            for column in range(group.num_columns):
                if group.column(column).file_path:
                    raise ValueError("external Parquet columns are forbidden")
        actual = source.read(use_threads=False)
        if actual.nbytes > MAX_PARQUET_BYTES or not actual.equals(expected, check_metadata=True):
            raise ValueError("source Parquet values differ from replayed source evidence")
