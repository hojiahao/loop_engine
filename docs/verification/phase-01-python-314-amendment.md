# Phase 1 amendment verification: Python 3.14 uv workspace

- Date: 2026-09-07 (Asia/Shanghai)
- Branch: `refactor/us-equities-loop-runtime`
- Status: complete; implementation `0615d81` pushed and all exit gates passed
- Decision: `docs/adr/0005-python-314-uv-workspace.md`

## Scope

This amendment replaces the original three Python 3.12 project environments and
lockfiles with:

- CPython 3.14.4 pinned by the root `.python-version`, bootstrap, CI, and the
  development image;
- one uv workspace containing the virtual legacy root, `loop_research`, and
  `loop_protocol`;
- one root `uv.lock` and one Git-ignored root `.venv`; and
- package-specific command wrappers that preserve each package's working
  directory while resolving the same workspace environment.

The historical Phase 1 record remains unchanged evidence of its original
3.12.13 run. This document records the later amendment.

## Dependency resolution

`./scripts/uv.sh lock --python 3.14.4` resolved 40 packages. The root lock
records `requires-python = "==3.14.*"` and the three workspace members:

- `loop-engine-workspace==0.2.0a1`;
- `loop-engine-protocol==0.2.0a1`; and
- `loop-research==0.2.0a1`.

The local Linux x86-64 environment installed binary distributions for the
critical compiled dependencies, including grpcio 1.83.1, grpcio-tools 1.83.1,
pydantic-core 2.46.5, NumPy 2.5.2, pandas 3.0.5, DuckDB 1.5.5, PyArrow 25.0.1,
mypy 2.3.1, and Ruff 0.16.6.

## Environment identity

`scripts/verify-python-environment.sh` verifies both the exact interpreter and
the resolved environment prefix. The host result is:

```text
CPython 3.14.4 uses the single root .venv.
```

Both package wrappers report the same executable:

```text
/home/hojiahao/loop_engine/.venv/bin/python3
```

No `.venv-legacy`, `.venv-research`, `.venv-protocol`, or package-local
environment remains. The container uses a separate named volume mounted at
`/workspace/.venv`, preserving this path contract without reusing host
interpreter links.

## Preliminary compatibility evidence

The Python migration itself has passed these focused checks:

| Check | Result |
| --- | --- |
| `uv lock --check` | passed; 40 locked packages |
| `loop_research` pytest | 1 passed |
| `loop_protocol` pytest | 206 passed |
| frozen legacy pytest | 216 passed, 1 skipped, 12 existing NumPy warnings |
| cross-language wire fixture freshness | passed |
| Compose configuration rendering | passed |

The Python wire producer now records `python==3.14.4`. All three producer wires
still have SHA-256
`15687e24b490fe1051b5e642bc55607e64969680e035b2ccf1b6d89017ad9ccd`.
The Protobuf current and baseline descriptor SHA-256 remains
`27a38398e290caee3fb857063c0f2adbbe43a7d2d44322103dbf4b72535be979`;
the interpreter migration did not advance the wire compatibility baseline.

## Final exit evidence

The implementation checkpoint `ed85d2d` and contributor-rule follow-up
`9ba4a36` were pushed to the refactor branch. GitHub Actions run
`34101082316` passed the Rust and TypeScript jobs and all three Python test
steps (including 236 protocol tests), but the Python jobs failed in the
setup-uv post-job cache step. The action's default temporary cache path did
not match `scripts/uv.sh`, which writes to `.tools/uv-cache`. All four
setup-uv callers now explicitly select that workspace cache path. No test or
cache-failure check is suppressed. The correction passed all seven jobs in
[GitHub Actions run 34101687394](https://github.com/hojiahao/loop_engine/actions/runs/34101687394)
for pushed commit `0615d818e4e0e7fdf404db02ee9ca1c5e0683889`.

The amended development image builds successfully with DaoCloud base images.
Its Rust component archives are individually SHA-256 pinned and rechecked in
a network-disabled installation layer. Verified image index digest:
`6ee2ce637026655e0cd2ef829a9682df1ea567d4d30355a9a05bdbffa5fc0451`.
Host bootstrap has also exercised direct component installation and subsequent
standalone-prefix validation. Final verification on 2026-09-07 was:

| Gate | Result |
| --- | --- |
| host `just check` / `test` / `build` / `doctor` | all passed |
| host `uv lock --check --offline` | passed; 40 packages |
| exact interpreter and root environment | CPython 3.14.4, single root `.venv` |
| final Python protocol tests | 236 passed |
| final research tests | 1 passed |
| final isolated legacy tests | 216 passed, 1 skipped, 12 NumPy warnings |
| remote isolated Python package tests and cache post-steps | all passed |
| clean DaoCloud container build, bootstrap, check, test, build, recheck, doctor | passed in CI job `101677333027` |
| implementation commit and push | `0615d81`, synchronized with origin |
| complete remote CI | 7/7 jobs passed in run `34101687394` |

The final clean-container sequence ran on a fresh GitHub-hosted Ubuntu 24.04
runner with unique Compose volumes. This is the clean-container evidence;
the local host was separately verified without unnecessarily repeating the
same container sequence. The original failing run remains part of the record.

No production market-data or factor-performance claim follows from a toolchain
compatibility run.
