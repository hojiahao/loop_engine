import os
import tempfile

import pytest


@pytest.fixture(scope="session", autouse=True)
def vendor_cache():
    with tempfile.TemporaryDirectory(prefix="loop-engine-zipline-tests-") as path:
        previous = {key: os.environ.get(key) for key in ("ZIPLINE_ROOT", "MPLCONFIGDIR")}
        os.environ.update(ZIPLINE_ROOT=path, MPLCONFIGDIR=path)
        yield
        for key, value in previous.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value
