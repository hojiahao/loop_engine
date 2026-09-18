import os
from pathlib import Path

import pytest

from loop_alphalens import artifacts
from loop_alphalens.artifacts import Deadline, Store, decode


def test_atomic_replay(tmp_path: Path) -> None:
    tmp_path.chmod(0o700)
    store = Store(tmp_path)
    first = store.publish(b"evidence")
    before = (tmp_path / first.sha256[7:]).stat().st_mtime_ns
    assert store.publish(b"evidence") == first
    assert (tmp_path / first.sha256[7:]).stat().st_mtime_ns == before
    assert store.read(first) == b"evidence"
    assert not list(tmp_path.glob(".loop-alphalens-*"))


def test_corruption_denied(tmp_path: Path) -> None:
    tmp_path.chmod(0o700)
    store = Store(tmp_path)
    ref = store.publish(b"original")
    path = tmp_path / ref.sha256[7:]
    path.write_bytes(b"modified")
    with pytest.raises(ValueError):
        store.publish(b"original")
    assert path.read_bytes() == b"modified"
    assert not list(tmp_path.glob(".loop-alphalens-*"))


def test_symlink_denied(tmp_path: Path) -> None:
    tmp_path.chmod(0o700)
    store = Store(tmp_path)
    ref = store.publish(b"original")
    path = tmp_path / ref.sha256[7:]
    target = tmp_path / "target"
    path.rename(target)
    path.symlink_to(target)
    with pytest.raises(OSError):
        store.read(ref)


def test_root_identity(tmp_path: Path) -> None:
    root = tmp_path / "store"
    root.mkdir(mode=0o700)
    store = Store(root)
    os.rename(root, tmp_path / "old")
    root.mkdir(mode=0o700)
    with pytest.raises(ValueError, match="replaced"):
        store.publish(b"content")


@pytest.mark.parametrize("content", [b'{"a":1,"a":2}', b'{"a":NaN}', b"[]"])
def test_invalid_json(content: bytes) -> None:
    with pytest.raises(ValueError):
        decode(content)


def test_deadline_bounds() -> None:
    for value in (0, -1, 181, float("nan"), float("inf")):
        with pytest.raises(ValueError):
            Deadline(value)


@pytest.mark.parametrize("ticks", [(2.0, 3.0, 2.5), (0.0, 1.0, 180.0)])
def test_deadline_failure(ticks: tuple[float, ...], monkeypatch: pytest.MonkeyPatch) -> None:
    values = iter(ticks)
    monkeypatch.setattr(artifacts.time, "monotonic", lambda: next(values))
    deadline = Deadline()
    deadline.check()
    with pytest.raises(TimeoutError):
        deadline.check()
