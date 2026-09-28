import csv
import hashlib
import io
import json
import os
from collections.abc import Iterator
from datetime import UTC, date, datetime
from pathlib import Path
from typing import Any

import numpy as np
import pytest

from loop_research.calendar import require_session_decisions
from loop_research.panel_io import ContentRef, load_panel

START = date(2010, 1, 4)
END = date(2010, 1, 6)


def timestamp(day: int) -> int:
    return int(datetime(2010, 1, day, 21, 5, tzinfo=UTC).timestamp()) * 1000


@pytest.fixture
def view(tmp_path: Path) -> Iterator[Path]:
    directory = tmp_path / "view"
    directory.mkdir()
    yield directory
    directory.chmod(0o700)


def publish(view: Path, content: bytes) -> ContentRef:
    view.chmod(0o700)
    digest = "sha256:" + hashlib.sha256(content).hexdigest()
    path = view / digest[7:]
    path.write_bytes(content)
    path.chmod(0o444)
    view.chmod(0o555)
    return ContentRef(sha256=digest, byte_size=len(content))


def rows() -> list[list[str]]:
    return [
        [f"2010-01-0{day}", security, "1", str(timestamp(day) - 60_000), str(day * 2)]
        for day in (4, 5, 6)
        for security in ("US.001", "US.002")
    ]


def declaration(view: Path, data: list[list[str]] | None = None) -> dict[str, Any]:
    output = io.StringIO(newline="")
    writer = csv.writer(output, lineterminator="\n")
    writer.writerow(["session", "security_id", "eligible", "known_at_ms", "market.close"])
    writer.writerows(rows() if data is None else data)
    values = publish(view, output.getvalue().encode("ascii"))
    return {
        "schema": "loop.factor-panel/v1",
        "quality": "synthetic",
        "sessions": ["2010-01-04", "2010-01-05", "2010-01-06"],
        "securities": ["US.001", "US.002"],
        "fields": ["market.close"],
        "decision_times_ms": [timestamp(day) for day in (4, 5, 6)],
        "evaluation_start": "2010-01-04",
        "values": values.model_dump(),
    }


def manifest(view: Path, document: dict[str, Any]) -> ContentRef:
    return publish(view, json.dumps(document, separators=(",", ":")).encode("ascii"))


# Scenario: actual read only files form the exact panel.
def test_files_form(view: Path) -> None:
    reference = manifest(view, declaration(view))
    loaded = load_panel(view, reference, sample_start=START, sample_end=END)
    np.testing.assert_array_equal(loaded.panel.fields["market.close"], [[8, 8], [10, 10], [12, 12]])
    assert loaded.panel.eligible.all()
    assert loaded.manifest.quality == "synthetic"
    loaded.check()


# Scenario: csv missingness does not become zero.
def test_csv_missingness(view: Path) -> None:
    data = rows()
    data[0][3:] = ["", ""]
    loaded = load_panel(
        view, manifest(view, declaration(view, data)), sample_start=START, sample_end=END
    )
    assert np.isnan(loaded.panel.fields["market.close"][0, 0])
    assert loaded.panel.eligible[0, 0]


@pytest.mark.parametrize("token", ["nan", "inf", "-inf", " 1", "1_0", "1e400"])
# Scenario: invalid observations fail closed.
def test_invalid_observations(view: Path, token: str) -> None:
    data = rows()
    data[0][-1] = token
    with pytest.raises(ValueError, match="observation"):
        load_panel(
            view, manifest(view, declaration(view, data)), sample_start=START, sample_end=END
        )


# Scenario: future observation is rejected.
def test_future_observation(view: Path) -> None:
    data = rows()
    data[0][3] = str(timestamp(4) + 1)
    with pytest.raises(ValueError, match="unavailable"):
        load_panel(
            view, manifest(view, declaration(view, data)), sample_start=START, sample_end=END
        )


# Scenario: observation without visibility is rejected.
def test_observation_visibility(view: Path) -> None:
    data = rows()
    data[0][3] = ""
    with pytest.raises(ValueError, match="visibility"):
        load_panel(
            view, manifest(view, declaration(view, data)), sample_start=START, sample_end=END
        )


@pytest.mark.parametrize("change", ["missing", "duplicate", "reordered", "additional"])
# Scenario: grid mismatch cannot change coverage.
def test_grid_mismatch(view: Path, change: str) -> None:
    data = rows()
    if change == "missing":
        data.pop()
    elif change == "duplicate":
        data[2] = data[1]
    elif change == "reordered":
        data[0], data[1] = data[1], data[0]
    else:
        data.append(data[-1])
    with pytest.raises(ValueError, match="grid"):
        load_panel(
            view, manifest(view, declaration(view, data)), sample_start=START, sample_end=END
        )


# Scenario: manifest cannot expand a frozen sample.
def test_manifest_expand(view: Path) -> None:
    with pytest.raises(ValueError, match="evaluation window"):
        load_panel(
            view, manifest(view, declaration(view)), sample_start=START, sample_end=date(2010, 1, 5)
        )


# Scenario: manifest cannot trim a frozen sample.
def test_manifest_trim(view: Path) -> None:
    with pytest.raises(ValueError, match="complete frozen"):
        load_panel(
            view, manifest(view, declaration(view)), sample_start=START, sample_end=date(2010, 1, 7)
        )


# Scenario: protected samples are not development inputs.
def test_protected_samples(view: Path) -> None:
    with pytest.raises(ValueError, match="development sample"):
        load_panel(
            view,
            manifest(view, declaration(view)),
            sample_start=date(2025, 1, 1),
            sample_end=date(2025, 1, 6),
        )


# Scenario: production quality cannot be claimed.
def test_production_quality(view: Path) -> None:
    document = declaration(view)
    document["quality"] = "production"
    with pytest.raises(ValueError):
        load_panel(view, manifest(view, document), sample_start=START, sample_end=END)


# Scenario: corrupted bytes are not loaded.
def test_corrupted_bytes(view: Path) -> None:
    document = declaration(view)
    reference = manifest(view, document)
    payload = view / document["values"]["sha256"][7:]
    payload.chmod(0o644)
    payload.write_bytes(b"x" * document["values"]["byte_size"])
    payload.chmod(0o444)
    with pytest.raises(ValueError, match="checksum"):
        load_panel(view, reference, sample_start=START, sample_end=END)


# Scenario: changed file invalidates loaded input.
def test_changed_file(view: Path) -> None:
    document = declaration(view)
    loaded = load_panel(view, manifest(view, document), sample_start=START, sample_end=END)
    payload = view / document["values"]["sha256"][7:]
    payload.chmod(0o644)
    with pytest.raises(ValueError, match="changed"):
        loaded.check()


# Scenario: symlink payload is rejected.
def test_symlink_payload(view: Path) -> None:
    document = declaration(view)
    reference = manifest(view, document)
    payload = view / document["values"]["sha256"][7:]
    view.chmod(0o755)
    replacement = view.parent / "untrusted"
    payload.rename(replacement)
    payload.symlink_to(replacement)
    view.chmod(0o555)
    with pytest.raises(OSError):
        load_panel(view, reference, sample_start=START, sample_end=END)


# Scenario: fifo payload is rejected without blocking.
def test_fifo_payload(view: Path) -> None:
    document = declaration(view)
    reference = manifest(view, document)
    payload = view / document["values"]["sha256"][7:]
    view.chmod(0o755)
    payload.unlink()
    os.mkfifo(payload, 0o444)
    view.chmod(0o555)
    with pytest.raises(ValueError, match="regular file"):
        load_panel(view, reference, sample_start=START, sample_end=END)


# Scenario: read write view is not a prepared worker mount.
def test_view_prepared(view: Path) -> None:
    reference = manifest(view, declaration(view))
    view.chmod(0o755)
    with pytest.raises(ValueError, match="read-only"):
        load_panel(view, reference, sample_start=START, sample_end=END)


# Scenario: decision before close is rejected.
def test_decision_close() -> None:
    with pytest.raises(ValueError, match="precedes"):
        require_session_decisions((START,), (timestamp(4) - 6 * 60_000,))


# Scenario: half day uses real scheduled close.
def test_half_day() -> None:
    session = date(2010, 11, 26)
    decision = int(datetime(2010, 11, 26, 18, 5, tzinfo=UTC).timestamp()) * 1000
    assert require_session_decisions((session,), (decision,)).name == "XNYS"


# Scenario: next day decision cannot hide future data.
def test_next_day() -> None:
    with pytest.raises(ValueError, match="another session"):
        require_session_decisions((START,), (timestamp(5),))
