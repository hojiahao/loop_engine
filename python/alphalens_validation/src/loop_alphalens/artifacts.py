"""Bounded private CAS IO for the independent process; no remote resolution."""

import hashlib
import json
import math
import os
import stat
import time
import uuid
from pathlib import Path
from typing import Any

from loop_alphalens.models import Reference


class Deadline:
    def __init__(self, seconds: float = 180) -> None:
        if not math.isfinite(seconds) or not 0 < seconds <= 180:
            raise ValueError("independent deadline bound")
        self.seconds = seconds
        self.started = self.previous = time.monotonic()

    def check(self) -> None:
        current = time.monotonic()
        if current < self.previous or current - self.started >= self.seconds:
            raise TimeoutError("independent deadline or clock regression")
        self.previous = current


def encode(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode(
        "ascii"
    )


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def _constant(value: str) -> None:
    raise ValueError("nonfinite JSON constant: " + value)


def decode(content: bytes) -> dict[str, Any]:
    value = json.loads(content, object_pairs_hook=_pairs, parse_constant=_constant)
    if not isinstance(value, dict):
        raise ValueError("JSON object required")
    return value


def reference(content: bytes) -> Reference:
    return Reference(sha256="sha256:" + hashlib.sha256(content).hexdigest(), byte_size=len(content))


def _version(value: os.stat_result) -> tuple[int, ...]:
    return value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns


def read_file(path: Path, limit: int, *, allow_empty: bool = False) -> bytes:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or not int(not allow_empty) <= before.st_size <= limit:
            raise ValueError("bounded regular artifact required")
        content = stream.read(before.st_size + 1)
        if (
            len(content) != before.st_size
            or _version(before) != _version(os.fstat(stream.fileno()))
            or _version(before) != _version(path.stat(follow_symlinks=False))
        ):
            raise ValueError("artifact changed during read")
    return content


class Store:
    def __init__(self, root: Path) -> None:
        metadata = root.stat(follow_symlinks=False)
        if (
            not root.is_absolute()
            or root.resolve(strict=True) != root
            or not stat.S_ISDIR(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o700
            or metadata.st_uid != os.geteuid()
        ):
            raise ValueError("private canonical runtime-owned store required")
        self.root = root
        self.identity = metadata.st_dev, metadata.st_ino

    def check(self) -> None:
        metadata = self.root.stat(follow_symlinks=False)
        if (
            self.identity != (metadata.st_dev, metadata.st_ino)
            or not stat.S_ISDIR(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o700
            or metadata.st_uid != os.geteuid()
        ):
            raise ValueError("independent store replaced")

    def read(self, ref: Reference) -> bytes:
        self.check()
        content = read_file(self.root / ref.sha256[7:], ref.byte_size)
        if reference(content) != ref:
            raise ValueError("independent artifact checksum differs")
        self.check()
        return content

    def document(self, digest: str) -> tuple[Reference, bytes]:
        ref = Reference(sha256=digest, byte_size=1)
        self.check()
        content = read_file(self.root / ref.sha256[7:], 128 * 1024)
        ref = reference(content)
        if ref.sha256 != digest:
            raise ValueError("independent manifest checksum differs")
        return ref, self.read(ref)

    def publish(self, content: bytes) -> Reference:
        self.check()
        ref = reference(content)
        temporary = self.root / (".loop-alphalens-" + uuid.uuid4().hex)
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(content)
                stream.flush()
                os.fsync(stream.fileno())
            self.check()
            try:
                os.link(temporary, self.root / ref.sha256[7:], follow_symlinks=False)
            except FileExistsError:
                self.read(ref)
            directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            temporary.unlink()
        self.check()
        return ref
