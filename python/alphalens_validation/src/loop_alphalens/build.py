"""Installed numerical byte identity, without claiming hermetic OS attestation."""

import hashlib
import importlib.metadata
import platform
import sys
import sysconfig
from pathlib import Path

from loop_alphalens.artifacts import Deadline, encode, read_file


def describe(deadline: Deadline) -> bytes:
    files: dict[str, Path] = {}
    versions = {}
    for name in ("alphalens-reloaded", "numpy", "pandas", "scipy", "pydantic", "pydantic-core"):
        package = importlib.metadata.distribution(name)
        versions[name] = package.version
        if package.files is None:
            raise ValueError("missing validator distribution file manifest")
        for item in package.files:
            label = str(item)
            if label.endswith(".py") or ".so" in label or label.endswith("/METADATA"):
                files[name + "/" + label] = Path(str(package.locate_file(item)))
    if (
        versions
        != {
            "alphalens-reloaded": "0.4.6",
            "numpy": "2.5.2",
            "pandas": "2.3.3",
            "scipy": "1.18.1",
            "pydantic": "2.13.5",
            "pydantic-core": "2.46.5",
        }
        or platform.python_version() != "3.14.4"
    ):
        raise ValueError("validator runtime differs from the tested profile")
    root = Path(__file__).parent
    for path in sorted(root.glob("*.py")):
        files["loop_alphalens/" + path.name] = path
    files["python-executable"] = Path(sys.executable).resolve(strict=True)
    library = Path(str(sysconfig.get_config_var("LIBDIR"))) / str(
        sysconfig.get_config_var("LDLIBRARY")
    )
    if library.is_file():
        files["python-library"] = library.resolve(strict=True)
    if len(files) > 8192:
        raise ValueError("validator source file budget")
    records = []
    total = 0
    for name, path in sorted(files.items()):
        deadline.check()
        content = read_file(path, 128 * 1024 * 1024, allow_empty=True)
        total += len(content)
        if total > 1_073_741_824:
            raise ValueError("validator source byte budget")
        records.append(
            {
                "name": name,
                "object": {
                    "sha256": "sha256:" + hashlib.sha256(content).hexdigest(),
                    "byte_size": len(content),
                },
            }
        )
    return encode(
        {
            "schema": "loop.alphalens-build/v1",
            "python": platform.python_version(),
            "versions": versions,
            "files": records,
        }
    )
