import os
import tempfile

import pytest

_cache = tempfile.TemporaryDirectory(prefix="loop-engine-alphalens-tests-")
os.environ["MPLCONFIGDIR"] = _cache.name
os.environ["MPLBACKEND"] = "Agg"
os.environ["OPENBLAS_NUM_THREADS"] = "1"


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    _cache.cleanup()
