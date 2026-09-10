# Durable window perturbation

The internal Rust `PerturbationRepository::advance_perturbation` command invokes
the actual installed Python worker through `PythonPerturber`. The command accepts
an existing source backtest, immutable IS family context and expected revision;
it does not accept scores, arbitrary executable paths or database URLs from users.
See ADR 0016 for numerical semantics, authority, bounds and rollback.

## Reproduce The Delivery Tests

After the ordinary workspace bootstrap:

```bash
bash scripts/uv.sh sync --all-packages --all-groups --locked
bash scripts/uv-research.sh run --isolated --locked pytest tests/test_perturbation.py
bash scripts/postgres-test.sh start
bash scripts/cargo.sh test -p loopd --test durable_perturbation --locked --offline -- --test-threads=1
bash scripts/cargo.sh test -p loopd --test durable_processes --locked --offline -- --test-threads=1
bash scripts/cargo.sh test -p loopd --lib killed_writer_preserves_atomicity --locked --offline -- --test-threads=1
bash scripts/postgres-test.sh stop
just check
```

The test database is TLS-enabled, loopback-only and disposable. Fixture helpers
refuse a database/principal other than `loop_engine_test`. The test Python path
is the existing repository-root `.venv/bin/python`; no extra persistent virtual
environment or production database is created. The Rust CI job explicitly
installs this worker before integration tests. On small hosts, finish compilation
before starting another heavy gate; deadlines remain active during tests.

The Python module also provides a low-level single-envelope worker entry point:

```bash
.venv/bin/python -I -m loop_research.perturbation < work.binpb > step.binpb
```

`work.binpb` is a `loop.v1.PerturbationWork`, and stdout is a
`loop.v1.PerturbationStep`. Inputs/outputs are capped at 1 MiB. Invalid envelopes
return exit code 2 with a redacted diagnostic; numerical functions never persist
state. Calling the module directly is a diagnostic, not authorized research,
durable optimization, an approved proposal or an external dispatch command.

## Operational Meaning

- `EXPLORATION`: a new eligible candidate, including cold start.
- `GRADIENT`: a new eligible candidate chosen using registered IS Sharpe history.
- `EXHAUSTED`: no candidate remains; stop this family instead of looping forever.
- `PreviouslyRejected`: evidence appeared during computation; no state committed.
- `RevisionConflict`: another command won; read its receipt/state before deciding
  on a new command. Do not blindly retry with an incremented revision.
- Infrastructure, stale evidence and authorization errors never create rejection
  memory or factor-admission decisions.

Retries of an accepted command return the original proposal with `replayed=true`.
That flag does not authorize a second evaluation. Queueing and retrying evaluation
jobs use the existing command/lease boundary and will be owned by Phase 11.
No public RPC, Web/TUI optimization button or production IS resolver is enabled
by this delivery. The remaining Phase 4 integrations are still required.
