"""Real-file build evidence and bounded publication failure paths."""

import hashlib
import os
from pathlib import Path

import pytest

from loop_research import build_identity as builds


def test_canonical_field_order() -> None:
    manifest: builds.FileManifest = {
        "schema": "loop.source-files/v1",
        "files": [
            {"name": "worker.py", "object": {"sha256": "sha256:" + "0" * 64, "byte_size": 1}}
        ],
    }
    assert builds.canonical_bytes(manifest).startswith(
        b'{"schema":"loop.source-files/v1","files":[{"name":"worker.py","object":{"sha256":'
    )


def test_real_file_bytes_change_identity(tmp_path: Path) -> None:
    path = tmp_path / "worker.py"
    path.write_bytes(b"source-v1")
    first = builds._Capture(None).file("worker.py", path)
    path.write_bytes(b"source-v2")
    second = builds._Capture(None).file("worker.py", path)
    assert first["object"]["sha256"] == "sha256:" + hashlib.sha256(b"source-v1").hexdigest()
    assert first != second


def test_source_tree_skips_only_bytecode(tmp_path: Path) -> None:
    (tmp_path / "worker.py").write_bytes(b"source")
    (tmp_path / "__pycache__").mkdir()
    (tmp_path / "__pycache__" / "worker.pyc").write_bytes(b"generated")
    (tmp_path / "worker.pyi").write_bytes(b"annotation")
    result = builds._Capture(None).tree("package", tmp_path)
    assert [entry["name"] for entry in result] == ["package/worker.py", "package/worker.pyi"]


def test_source_symlink_is_rejected(tmp_path: Path) -> None:
    (tmp_path / "real.py").write_bytes(b"source")
    (tmp_path / "alias.py").symlink_to(tmp_path / "real.py")
    with pytest.raises(OSError):
        builds._Capture(None).file("alias.py", tmp_path / "alias.py")


def test_directory_symlink_is_rejected(tmp_path: Path) -> None:
    (tmp_path / "real").mkdir()
    (tmp_path / "alias").symlink_to(tmp_path / "real", target_is_directory=True)
    with pytest.raises(ValueError, match="directory symlink"):
        builds._Capture(None).tree("package", tmp_path)


def test_fifo_cannot_block_capture(tmp_path: Path) -> None:
    path = tmp_path / "pipe"
    os.mkfifo(path)
    with pytest.raises(ValueError, match="regular files"):
        builds._Capture(None).file("pipe", path)


def test_capture_has_byte_budget(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    path = tmp_path / "large"
    path.write_bytes(b"too large")
    monkeypatch.setattr(builds, "MAX_BYTES", 4)
    with pytest.raises(ValueError, match="bounded regular files"):
        builds._Capture(None).file("large", path)


def test_capture_has_count_budget(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(builds, "MAX_FILES", 1)
    capture = builds._Capture(None)
    capture.content("first", b"1")
    with pytest.raises(ValueError, match="bounds"):
        capture.content("second", b"2")


def test_capture_has_deadline(monkeypatch: pytest.MonkeyPatch) -> None:
    capture = builds._Capture(None)
    monkeypatch.setattr(builds.time, "monotonic", lambda: capture.started + builds.MAX_SECONDS + 1)
    with pytest.raises(ValueError, match="timed out"):
        capture.content("late", b"data")


def test_publication_is_immutable_and_cleans_temporary_files(tmp_path: Path) -> None:
    descriptor = os.open(tmp_path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        first = builds._Capture(descriptor).content("source", b"content")
        second = builds._Capture(descriptor).content("source", b"content")
    finally:
        os.close(descriptor)
    assert first == second
    assert [path.name for path in tmp_path.iterdir()] == [first["object"]["sha256"][7:]]


def test_existing_wrong_bytes_are_not_overwritten(tmp_path: Path) -> None:
    expected = builds._reference(b"correct")
    path = tmp_path / expected["sha256"][7:]
    path.write_bytes(b"invalid")
    descriptor = os.open(tmp_path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        with pytest.raises(ValueError, match="checksum"):
            builds._Capture(descriptor).content("source", b"correct")
    finally:
        os.close(descriptor)
    assert path.read_bytes() == b"invalid"
    assert not list(tmp_path.glob(".loop-build-*"))


def test_existing_symlink_is_not_followed(tmp_path: Path) -> None:
    expected = builds._reference(b"correct")
    outside = tmp_path / "untouched"
    outside.write_bytes(b"correct")
    (tmp_path / expected["sha256"][7:]).symlink_to(outside)
    descriptor = os.open(tmp_path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        with pytest.raises(OSError):
            builds._Capture(descriptor).content("source", b"correct")
    finally:
        os.close(descriptor)
    assert outside.read_bytes() == b"correct"
    assert not list(tmp_path.glob(".loop-build-*"))


def test_relative_store_is_rejected() -> None:
    with pytest.raises(ValueError, match="absolute"):
        builds.describe_build(Path("relative-store"))


def test_installed_build_is_repeatable() -> None:
    identity = builds.describe_build()
    assert (
        builds.require_build(identity.source["sha256"], identity.environment["sha256"]) == identity
    )


def test_wrong_installed_build_is_rejected() -> None:
    with pytest.raises(ValueError, match="frozen context"):
        builds.require_build("sha256:" + "0" * 64, "sha256:" + "0" * 64)
