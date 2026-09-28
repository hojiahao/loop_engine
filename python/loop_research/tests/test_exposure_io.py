"""Byte-valid exposure files still require causal, aligned and finite observations."""

import hashlib
from datetime import date
from pathlib import Path

import numpy as np
import pytest

from loop_research.data.fetch_records import CachedObject
from loop_research.exposure_io import load_exposures

HEADER = "session,security_id,known_at_ms,industry,market_cap,beta\n"
ROW = "2010-01-04,US.001,1262639040000,A,100,1\n"


def read(directory: Path, rows: str) -> None:
    content = (HEADER + rows).encode("ascii")
    digest = hashlib.sha256(content).hexdigest()
    directory.mkdir(mode=0o700)
    path = directory / digest
    path.write_bytes(content)
    path.chmod(0o444)
    directory.chmod(0o555)
    try:
        actual, guard = load_exposures(
            directory,
            CachedObject(sha256="sha256:" + digest, byte_size=len(content)),
            sessions=(date(2010, 1, 4),),
            securities=("US.001",),
            decisions=(1262639100000,),
        )
        assert actual.industry == (("A",),)
        np.testing.assert_array_equal(actual.market_cap, [[100]])
        guard.check()
        path.chmod(0o644)
        path.write_bytes(content + b"corrupt")
        with pytest.raises(ValueError, match="changed after verification"):
            guard.check()
    finally:
        directory.chmod(0o700)


# Scenario: verified exposures retain live file guard.
def test_exposures_retain(tmp_path: Path) -> None:
    read(tmp_path / "view", ROW)


@pytest.mark.parametrize(
    "rows",
    [
        ROW.replace("1262639040000", "1262639100001"),
        ROW.replace("1262639040000", ""),
        ROW.replace("1262639040000", "01262639040000"),
        ROW.replace("US.001", "US.002"),
        ROW.replace(",100,", ",-100,"),
        ROW.replace(",100,", ",1e309,"),
        ROW + ROW,
        "",
    ],
)
# Scenario: hashed exposure still needs valid observations.
def test_hashed_exposure(tmp_path: Path, rows: str) -> None:
    with pytest.raises(ValueError):
        read(tmp_path / "view", rows)
