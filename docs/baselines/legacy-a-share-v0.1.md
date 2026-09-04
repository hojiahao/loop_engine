# Legacy A-share baseline: v0.1

Recorded on 2026-09-04 before the US-equities refactor.

## Git identity

| Item | Value |
|---|---|
| Commit | `f3fd8bf816f733f2ec6dbb761870a8d6509c2909` |
| Git tree | `ac0f0ad557b59400555290a2ce47caf640d4600b` |
| Annotated tag | `legacy-a-share-v0.1` |
| Author | `hojiahao <hojiahao@outlook.com>` |

The annotated tag is the authoritative file manifest. The checksums below pin
the mutable research-state artifacts independently of Git object encoding.

## Artifact checksums

| Path | SHA-256 |
|---|---|
| `README.md` | `2d23d8f70e962d227b50131af5f6e12036030c5308a0c93453d5de296d742821` |
| `pyproject.toml` | `39a9e3880ba892525a5515c5d182547696dc8b24e34f437beede443fb020cf13` |
| `uv.lock` | `483bc571bc3acd873da56970eb6fbd28d2f6c1ab86d1d8e8a8b3df1867d1d607` |
| `output/checkpoint.json` | `b570d0e542330085ffb6c736196bbd1a8a1c78143c76b6768798d0ad0149cee0` |
| `output/failed_patterns.json` | `a32e50d70b26a1db405d7a3f80a911a574cb7dc3f030f4d5bb1e7cef7ff847d9` |
| `output/mined_patterns.json` | `2e8acd20113ef70eb447b5bbbc9f5f138a74933a0442f66a19a3505bc9a97748` |

## Verification baseline

Command:

```bash
UV_CACHE_DIR=/tmp/loop-engine-uv-cache uv run pytest
```

Result: 216 passed, 1 skipped, and 12 NumPy warnings in 14.35 seconds. The
skipped test requires the unavailable Windows AlphaLab fixture. The warnings
come from deliberately sparse orchestration test panels.

`code/lib_status.py` reported iteration 635, 38,133 tested expressions, 23
stored factors, and zero current metrics. All 23 stored metrics are stale. The
2025 values are contaminated development-validation evidence and are not OOS.

## Locked dependency snapshot

The complete dependency resolution is `uv.lock` at the checksum above. The
active environment contained Python 3.14.4, uv 0.11.29, DuckDB 1.5.5, NumPy
2.5.2, pandas 3.0.5, PyArrow 25.0.1, PyYAML 6.0.3, requests 2.34.2, and pytest
9.1.1. The refactor will use a separately pinned Python 3.12 environment.
