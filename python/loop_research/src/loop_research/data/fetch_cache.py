"""Private, bounded immutable cache IO using the existing artifact publisher."""

import hashlib
import os
import stat
from pathlib import Path

from loop_research.build_identity import publish_object
from loop_research.data.fetch_records import CachedObject


def _version(value: os.stat_result) -> tuple[int, int, int, int, int]:
    return value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns


def _read(descriptor: int, limit: int) -> bytes:
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode) or not 0 < before.st_size <= limit:
            raise ValueError("input must be a bounded nonempty regular file")
        content = stream.read(before.st_size + 1)
        if len(content) != before.st_size or _version(before) != _version(
            os.fstat(stream.fileno())
        ):
            raise ValueError("input changed during read")
        return content


def read_config_bytes(path: Path) -> bytes:
    """Read only a small regular configuration file, without following its leaf."""
    return _read(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 64 * 1024)


def private_directory(store: Path) -> int:
    """Open an existing canonical private directory; caller must close the FD."""
    if not store.is_absolute() or store.resolve(strict=True) != store:
        raise ValueError("cache directory must be absolute and canonical")
    descriptor = os.open(store, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    metadata = os.fstat(descriptor)
    if metadata.st_uid != os.geteuid() or metadata.st_mode & 0o777 != 0o700:
        os.close(descriptor)
        raise ValueError("cache requires a private runtime-owned directory")
    return descriptor


def publish(store: Path, content: bytes) -> CachedObject:
    """Publish using the existing no-overwrite CAS; corruption never gets repaired silently."""
    return CachedObject.model_validate(publish_object(store, content))


def read_cached(store: Path, reference: CachedObject) -> bytes:
    """Verify bounded bytes from a digest-derived filename; no URL/path resolution."""
    reference = CachedObject.model_validate(reference)
    directory = private_directory(store)
    try:
        name = reference.sha256[7:]
        before = os.stat(name, dir_fd=directory, follow_symlinks=False)
        content = _read(
            os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory),
            reference.byte_size,
        )
        if _version(before) != _version(os.stat(name, dir_fd=directory, follow_symlinks=False)):
            raise ValueError("cached file changed during read")
        if (
            len(content) != reference.byte_size
            or "sha256:" + hashlib.sha256(content).hexdigest() != reference.sha256
        ):
            raise ValueError("cached object digest differs")
        return content
    finally:
        os.close(directory)


def read_receipt(store: Path, digest: str) -> tuple[CachedObject, bytes]:
    """Resolve a user-supplied receipt digest, with an independent 128 KiB bound."""
    checked = CachedObject(sha256=digest, byte_size=1)
    directory = private_directory(store)
    try:
        size = os.stat(checked.sha256[7:], dir_fd=directory, follow_symlinks=False).st_size
        if not 0 < size <= 128 * 1024:
            raise ValueError("receipt exceeds its byte budget")
    finally:
        os.close(directory)
    reference = CachedObject(sha256=digest, byte_size=size)
    return reference, read_cached(store, reference)
