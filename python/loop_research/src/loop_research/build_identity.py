"""Byte-backed identity of the fixed local perturbation worker, not OS attestation."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import platform
import stat
import sys
import sysconfig
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import TypedDict

MAX_FILES = 8192
MAX_BYTES = 1_073_741_824
MAX_SECONDS = 8.0


class ObjectReference(TypedDict):
    """A content identity with an exact byte bound."""

    sha256: str
    byte_size: int


class NamedFile(TypedDict):
    """Stable logical name; never a URI, credential or import path override."""

    name: str
    object: ObjectReference


class FileManifest(TypedDict):
    """Canonical field order shared with the Rust manifest parser."""

    schema: str
    files: list[NamedFile]


@dataclass(frozen=True)
class BuildIdentity:
    """Source and installed numerical environment content references."""

    source: ObjectReference
    environment: ObjectReference


def canonical_bytes(value: object) -> bytes:
    """Emit ASCII JSON in the declared schema's field order."""
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode(
        "ascii"
    )


def _reference(content: bytes) -> ObjectReference:
    return {"sha256": "sha256:" + hashlib.sha256(content).hexdigest(), "byte_size": len(content)}


class _Capture:
    def __init__(self, directory: int | None) -> None:
        self.directory = directory
        self.started = time.monotonic()
        self.count = 0
        self.total = 0

    def _budget(self, size: int) -> None:
        self.count += 1
        self.total += size
        if self.count > MAX_FILES or self.total > MAX_BYTES:
            raise ValueError("worker build exceeds verification bounds")
        self.check_time()

    def check_time(self) -> None:
        if time.monotonic() - self.started > (MAX_SECONDS if self.directory is None else 60.0):
            raise ValueError("worker build verification timed out")

    def content(self, name: str, content: bytes) -> NamedFile:
        self._budget(len(content))
        reference = _reference(content)
        if self.directory is not None:
            _publish(self.directory, reference, content)
        return {"name": name, "object": reference}

    def file(self, name: str, path: Path) -> NamedFile:
        self.check_time()
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, "rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_BYTES - self.total:
                raise ValueError("worker build requires bounded regular files")
            content = stream.read(before.st_size + 1)
            after = os.fstat(stream.fileno())
            current = path.stat(follow_symlinks=False)
            if len(content) != before.st_size or _version(before) != _version(after):
                raise ValueError("worker file changed during verification")
            if _version(before) != _version(current):
                raise ValueError("worker file was replaced during verification")
        return self.content(name, content)

    def tree(self, prefix: str, root: Path) -> list[NamedFile]:
        result: list[NamedFile] = []
        if root.is_symlink() or not root.is_dir():
            raise ValueError("worker package must be a real directory")
        for parent, directories, files in os.walk(root, followlinks=False):
            self.check_time()
            directories[:] = sorted(name for name in directories if name != "__pycache__")
            for directory in directories:
                if (Path(parent) / directory).is_symlink():
                    raise ValueError("worker package contains a directory symlink")
            for filename in sorted(files):
                if filename.endswith((".pyc", ".pyo")):
                    continue
                path = Path(parent) / filename
                result.append(self.file(prefix + "/" + path.relative_to(root).as_posix(), path))
        return result


def _version(value: os.stat_result) -> tuple[int, int, int, int, int]:
    return value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns


def _package_root(name: str) -> Path:
    spec = importlib.util.find_spec(name)
    if spec is None or spec.submodule_search_locations is None:
        raise ValueError("required worker package is unavailable")
    paths = list(spec.submodule_search_locations)
    if len(paths) != 1:
        raise ValueError("worker package has ambiguous import roots")
    return Path(paths[0])


def _publish(directory: int, reference: ObjectReference, content: bytes) -> None:
    name = reference["sha256"][7:]
    temporary = ".loop-build-" + uuid.uuid4().hex
    descriptor = os.open(
        temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory
    )
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        try:
            os.link(
                temporary, name, src_dir_fd=directory, dst_dir_fd=directory, follow_symlinks=False
            )
        except FileExistsError:
            existing = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
            with os.fdopen(existing, "rb") as stream:
                metadata = os.fstat(stream.fileno())
                if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != len(content):
                    raise ValueError("existing build object is invalid") from None
                if _reference(stream.read(len(content) + 1)) != reference:
                    raise ValueError("existing build object checksum differs") from None
        os.fsync(directory)
    finally:
        os.unlink(temporary, dir_fd=directory)


def describe_build(store: Path | None = None) -> BuildIdentity:
    """Verify installed bytes; optionally publish immutable CAS objects.

    Covers the complete research/protocol packages, NumPy, Protobuf's Python and
    native runtime, NumPy's bundled native libraries, interpreter and shared
    Python library. Runtime ABI/platform facts are explicit. This does not claim
    a hermetic OS, validate market data or trust a caller-selected executable.
    Existing objects are checked, never overwritten. Callers own the store path.
    """
    directory = None
    if store is not None:
        if not store.is_absolute():
            raise ValueError("build artifact store must be absolute")
        directory = os.open(store, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        capture = _Capture(directory)
        source = []
        for package in ("loop_research", "loop_protocol", "loop"):
            source.extend(capture.tree(package, _package_root(package)))
        environment = []
        for package in ("numpy", "google.protobuf"):
            environment.extend(capture.tree(package.replace(".", "/"), _package_root(package)))
        numpy_libraries = _package_root("numpy").parent / "numpy.libs"
        if numpy_libraries.is_dir():
            environment.extend(capture.tree("numpy.libs", numpy_libraries))
        protobuf_native = importlib.util.find_spec("google._upb._message")
        if protobuf_native is None or protobuf_native.origin is None:
            raise ValueError("pinned Protobuf native runtime is unavailable")
        environment.append(capture.file("protobuf-native", Path(protobuf_native.origin)))
        # Venv interpreter links are resolved as deployment configuration. The
        # object records the actual executable's bytes, not the symlink text.
        environment.append(
            capture.file("python-executable", Path(sys.executable).resolve(strict=True))
        )
        library_dir = sysconfig.get_config_var("LIBDIR")
        library_name = sysconfig.get_config_var("LDLIBRARY")
        if isinstance(library_dir, str) and isinstance(library_name, str):
            library = Path(library_dir) / library_name
            if library.is_file():
                environment.append(capture.file("python-library", library.resolve(strict=True)))
        runtime = {
            "schema": "loop.python-runtime/v1",
            "version": sys.version,
            "implementation": sys.implementation.name,
            "cache_tag": sys.implementation.cache_tag,
            "soabi": sysconfig.get_config_var("SOABI"),
            "machine": platform.machine(),
            "system": platform.system(),
            "libc": list(platform.libc_ver()),
        }
        environment.append(capture.content("python-runtime.json", canonical_bytes(runtime)))
        references = []
        for role, files in (("source", source), ("environment", environment)):
            files.sort(key=lambda file: file["name"])
            if len({file["name"] for file in files}) != len(files):
                raise ValueError("duplicate build file name")
            manifest: FileManifest = {"schema": f"loop.{role}-files/v1", "files": files}
            references.append(
                capture.content(role + "-manifest", canonical_bytes(manifest))["object"]
            )
        return BuildIdentity(source=references[0], environment=references[1])
    finally:
        if directory is not None:
            os.close(directory)


def require_build(source_sha256: str, environment_sha256: str) -> BuildIdentity:
    """Reject drift from the service-pinned build before any numerical output."""
    identity = describe_build()
    if (
        identity.source["sha256"] != source_sha256
        or identity.environment["sha256"] != environment_sha256
    ):
        raise ValueError("installed worker build differs from the frozen context")
    return identity
