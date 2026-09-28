"""Byte identity for the pinned independent interpreter and numerical dependency closure."""

import hashlib
import importlib.metadata
import platform
import sys
import sysconfig
from pathlib import Path

from loop_zipline.artifacts import Deadline, encode, read_file


def read_source(path: Path, deadline: Deadline) -> bytes:
    """Retry at most two metadata races while uv links a shared dependency.

    Every attempt retains the strict inode/size/mtime/ctime checks. This does
    not relax CAS reads, cache earlier bytes or suppress changed build identity;
    the complete dependency snapshot is still recomputed before publication.
    """
    for attempt in range(3):
        deadline.check()
        try:
            return read_file(path, 128 * 1024 * 1024, allow_empty=True)
        except ValueError as error:
            if str(error) != "artifact changed during read" or attempt == 2:
                raise
    raise ValueError("independent source retry bound")


def describe(deadline: Deadline) -> bytes:
    versions = {}
    paths: dict[str, Path] = {}
    required = {
        "zipline-reloaded": "3.1.1",
        "bcolz-zipline": "1.2.10",
        "numpy": "2.5.2",
        "pandas": "2.3.3",
        "scipy": "1.18.1",
        "pydantic": "2.13.5",
        "setuptools": "80.9.0",
    }
    # Include native ledger/asset code and calendar/statistics dependency bytes,
    # not just an asserted source commit or a pip version string.
    names = (
        *required,
        "pydantic-core",
        "exchange-calendars",
        "empyrical-reloaded",
        "bottleneck",
        "numexpr",
        "python-dateutil",
        "pytz",
        "tzdata",
        "toolz",
        "intervaltree",
        "sortedcontainers",
    )
    for name in names:
        distribution = importlib.metadata.distribution(name)
        versions[name] = distribution.version
        if distribution.files is None:
            raise ValueError("missing independent distribution manifest")
        for item in distribution.files:
            label = str(item)
            if (
                label.endswith(".py")
                or ".so" in label
                or label.endswith("/METADATA")
                or "zoneinfo/" in label
            ):
                paths[name + "/" + label] = Path(str(distribution.locate_file(item)))
    if platform.python_version() != "3.12.13" or any(
        versions[name] != version for name, version in required.items()
    ):
        raise ValueError("independent runtime differs from tested profile")
    for path in sorted(Path(__file__).parent.glob("*.py")):
        paths["loop_zipline/" + path.name] = path
    paths["python-executable"] = Path(sys.executable).resolve(strict=True)
    library = Path(str(sysconfig.get_config_var("LIBDIR"))) / str(
        sysconfig.get_config_var("LDLIBRARY")
    )
    if library.is_file():
        paths["python-library"] = library.resolve(strict=True)
    if len(paths) > 16384:
        raise ValueError("independent source file budget")
    files = []
    total = 0
    for name, path in sorted(paths.items()):
        deadline.check()
        content = read_source(path, deadline)
        total += len(content)
        if total > 1024**3:
            raise ValueError("independent source byte budget")
        files.append(
            {
                "name": name,
                "sha256": "sha256:" + hashlib.sha256(content).hexdigest(),
                "byte_size": len(content),
            }
        )
    return encode(
        {
            "schema": "loop.zipline-build/v1",
            "python": platform.python_version(),
            "versions": versions,
            "files": files,
        }
    )
