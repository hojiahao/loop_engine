"""Real OS writer/crash acceptance for the reused immutable cache publisher."""

import hashlib
import json
import selectors
import subprocess
import sys
from pathlib import Path

import pytest

from loop_research.data.fetch_cache import publish, read_cached
from loop_research.data.fetch_records import CachedObject

WRITER = """
import json, sys
from pathlib import Path
from loop_research.data.fetch_cache import publish
store = Path(sys.argv[1])
common = publish(store, b"shared-source-evidence")
private = publish(store, ("writer-" + sys.argv[2]).encode())
print(json.dumps([common.model_dump(), private.model_dump()]), flush=True)
"""

CRASH_WRITER = """
import os, sys
from pathlib import Path
import loop_research.build_identity as builds
from loop_research.data.fetch_cache import publish
real_link = os.link
stage = sys.argv[2]
def fenced_link(*args, **kwargs):
    if stage == "after":
        real_link(*args, **kwargs)
    print("commit-boundary", flush=True)
    sys.stdin.buffer.read(1)
    if stage == "before":
        real_link(*args, **kwargs)
builds.os.link = fenced_link
publish(Path(sys.argv[1]), b"crash-evidence")
"""


@pytest.mark.parametrize("writers", [2, 4, 8])
# Scenario: independent processes preserve shared objects.
def test_independent_processes(tmp_path: Path, writers: int) -> None:
    processes = [
        subprocess.Popen(
            [sys.executable, "-I", "-c", WRITER, str(tmp_path), str(index)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        for index in range(writers)
    ]
    try:
        shared = set()
        for index, process in enumerate(processes):
            output, error = process.communicate(timeout=30)
            assert process.returncode == 0, error
            common, private = [CachedObject.model_validate(value) for value in json.loads(output)]
            shared.add(common.sha256)
            assert read_cached(tmp_path, common) == b"shared-source-evidence"
            assert read_cached(tmp_path, private) == ("writer-" + str(index)).encode()
        assert len(shared) == 1
        assert len(list(tmp_path.iterdir())) == writers + 1
        assert not list(tmp_path.glob(".loop-build-*"))
    finally:
        for process in processes:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)


@pytest.mark.parametrize("stage", ["before", "after"])
# Scenario: kill at publish commit can restart.
def test_kill_restart(tmp_path: Path, stage: str) -> None:
    process = subprocess.Popen(
        [sys.executable, "-I", "-c", CRASH_WRITER, str(tmp_path), stage],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    digest = hashlib.sha256(b"crash-evidence").hexdigest()
    try:
        assert process.stdout is not None
        with selectors.DefaultSelector() as ready:
            ready.register(process.stdout, selectors.EVENT_READ)
            assert ready.select(timeout=10), "publisher did not reach the commit boundary"
        assert process.stdout.readline() == b"commit-boundary\n"
        assert (tmp_path / digest).exists() == (stage == "after")
        process.kill()
        process.communicate(timeout=10)
        assert process.returncode == -9
        # Restart never treats an unfinished temp name as a complete object.
        reference = publish(tmp_path, b"crash-evidence")
        assert read_cached(tmp_path, reference) == b"crash-evidence"
        assert (tmp_path / digest).read_bytes() == b"crash-evidence"
        # A hard kill can orphan its unique temporary hard link. It is not
        # exposed as a receipt/object and is removed with this owned test dir.
        assert all(path.read_bytes() == b"crash-evidence" for path in tmp_path.iterdir())
    finally:
        if process.poll() is None:
            process.kill()
        process.communicate(timeout=10)
