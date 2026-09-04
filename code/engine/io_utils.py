# -*- coding: utf-8 -*-
"""Durable local persistence and single-writer process coordination."""
from __future__ import annotations

import json
import os
import socket
import tempfile
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


class AlreadyRunningError(RuntimeError):
    """Raised when another process already owns a command lock."""


def _fsync_directory(path: Path) -> None:
    """Persist a directory entry where the platform supports directory fsync."""
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    try:
        fd = os.open(path, flags)
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass
    finally:
        os.close(fd)


def atomic_write_text(path: str | Path, text: str) -> None:
    """Atomically replace *path* using a unique same-directory temporary file."""
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, raw_tmp = tempfile.mkstemp(
        prefix=f".{target.name}.", suffix=".tmp", dir=target.parent
    )
    tmp = Path(raw_tmp)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, target)
        _fsync_directory(target.parent)
    except BaseException:
        tmp.unlink(missing_ok=True)
        raise


def atomic_write_json(path: str | Path, value: Any, *, indent: int | None = None) -> None:
    payload = json.dumps(value, ensure_ascii=False, indent=indent)
    atomic_write_text(path, payload + "\n")


@contextmanager
def atomic_output_path(path: str | Path):
    """Yield a temporary path and atomically publish it after the writer closes."""
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    fd, raw_tmp = tempfile.mkstemp(
        prefix=f".{target.name}.", suffix=".tmp", dir=target.parent
    )
    os.close(fd)
    tmp = Path(raw_tmp)
    try:
        yield tmp
        with open(tmp, "rb") as handle:
            os.fsync(handle.fileno())
        os.replace(tmp, target)
        _fsync_directory(target.parent)
    except BaseException:
        tmp.unlink(missing_ok=True)
        raise


class ProcessLock:
    """Advisory, non-blocking single-writer lock held for a command lifetime."""

    def __init__(self, path: str | Path):
        self.path = Path(path)
        self._handle = None

    def acquire(self) -> "ProcessLock":
        if self._handle is not None:
            raise RuntimeError(f"lock already acquired: {self.path}")
        self.path.parent.mkdir(parents=True, exist_ok=True)
        handle = open(self.path, "a+", encoding="utf-8")
        try:
            self._lock_file(handle)
        except OSError as exc:
            handle.seek(0)
            owner = handle.read().strip() or "unknown owner"
            handle.close()
            raise AlreadyRunningError(
                f"another process owns {self.path}: {owner}"
            ) from exc

        metadata = {
            "pid": os.getpid(),
            "host": socket.gethostname(),
            "acquired_at": datetime.now(timezone.utc).isoformat(),
        }
        handle.seek(0)
        handle.truncate()
        handle.write(json.dumps(metadata, ensure_ascii=True))
        handle.flush()
        os.fsync(handle.fileno())
        self._handle = handle
        return self

    @staticmethod
    def _lock_file(handle) -> None:
        if os.name == "nt":
            import msvcrt

            handle.seek(0)
            if not handle.read(1):
                handle.write("\0")
                handle.flush()
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl

            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)

    @staticmethod
    def _unlock_file(handle) -> None:
        if os.name == "nt":
            import msvcrt

            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl

            fcntl.flock(handle.fileno(), fcntl.LOCK_UN)

    def release(self) -> None:
        if self._handle is None:
            return
        handle, self._handle = self._handle, None
        try:
            self._unlock_file(handle)
        finally:
            handle.close()

    def __enter__(self) -> "ProcessLock":
        return self.acquire()

    def __exit__(self, exc_type, exc, tb) -> None:
        self.release()


def checkpoint_lock_path(checkpoint_path: str | Path) -> Path:
    path = Path(checkpoint_path)
    return path.with_name(path.name + ".lock")
