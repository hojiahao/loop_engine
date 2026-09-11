# Trusted research manifests

Phase 4 unit 3 provides a file-backed development resolver, installed-worker
build verification and an audited JSON result writer. It is an internal command
integration, not a public research endpoint, a data feed or a primary backtester.
Runtime identity/storage isolation is unit 4; authorized AST execution is unit 5.

## Capture the installed worker

After `just bootstrap`, create an operator-owned artifact directory outside any
holdout storage. The directory must already exist and its absolute path must be
selected by the service operator, never by an Agent request. For example, with
an existing `/srv/loop-engine/development/objects` directory:

```bash
./scripts/uv-research.sh run --locked --offline --no-sync \
  loop-research build-manifests --store /srv/loop-engine/development/objects
```

The command prints source/environment `ObjectRef` values. Each has `sha256`
(`sha256:` plus 64 lowercase hex digits) and `byte_size`. The corresponding file
is stored directly under the object root using the 64 hex digits as its name.
No URI supplies a filesystem path. Existing objects are verified, not overwritten.
Publication uses a unique temporary file, fsync and no-clobber link; normal
failure removes that invocation's temporary file. A process kill may leave its
`.loop-build-*` temporary file, which an operator may remove only after confirming
no capture is running. Already published content objects remain immutable.

The capture covers all non-bytecode files in `loop_research`, `loop_protocol`,
generated `loop`, NumPy, Protobuf, bundled NumPy native libraries, native Protobuf,
the resolved interpreter, any present shared Python library, and ABI/platform
facts. It excludes `__pycache__`, `.pyc` and `.pyo`; symlinks within package trees
and special files are refused. This is not full OS/stdlib/dependency attestation.
Readonly deployments and dependency coverage for additional numerical workers
remain required; changing Python/NumPy/source bytes produces a different context.

## Pin the research context

The operator supplies `TrustedManifests::load` with an absolute `LocalArtifacts`
root, catalog `ObjectRef`, validated `loop-core` operator registries and explicitly
trusted reader identities. The host must authenticate those identities separately;
an `Actor` received in request metadata is not an authentication mechanism.
Readers are authorized to all development evidence in that pinned catalog.

JSON is UTF-8 with no extra whitespace, declared struct field order, and no
unknown or duplicate fields. Collections requiring ordering are strictly sorted
and duplicate-free. Exact-decimal metrics are strings, not JSON floating-point
numbers. This versioned internal byte format is separate from Protobuf transport
and from the existing canonical factor/AST format. Examples of complete synthetic
documents are constructed by `crates/loopd/src/manifests/tests/fixture.rs`.

| Document discriminator | Required bindings |
| --- | --- |
| `loop.research-catalog/v1` | Sorted job/spec/result/review entries and immutable current contexts |
| `loop.research-context/v1` | Source, registry, configuration, data, calendar, environment and optional window family |
| `loop.source-files/v1`, `loop.environment-files/v1` | Ordered logical names and actual object references |
| `loop.research-configuration/v1` | Backtest engine/version, policy IDs, revisions and actual policy documents |
| `loop.research-policy/v1` | Matching policy identity/revision and bounded settings |
| `loop.development-dataset/v1` | Sample role/dates, data grade, snapshots, entitlement and referenced artifacts |
| `loop.trading-calendar/v1` | XNYS, America/New_York and ordered session dates |
| `loop.factor-manifest/v1` | Canonical FactorSpec ID, specification bytes and AST bytes |
| `loop.backtest-spec/v1` | Context, factor, sample, engine/version, seed and simple NAV return definition |
| `loop.backtest-result/v1` | Source job/spec, engine/version, metrics, eight result-series references and completion time |
| `loop.artifact-schema/v1` | Schema name/version, media type and column names |
| `loop.window-family/v1` | Algorithm, RNG/backtest seeds and canonical candidates differing at one window |
| `loop.admission-review/v1` | Exact result/factor/policy/library, coverage, machine/semantic outcome and replacements |

Artifact schema **names** follow the existing protocol identifier rules, for
example `loop.backtest_result` or `loop.admission_review`; they are distinct from
the JSON discriminators above. Their immutable schema document must match exactly.
The eight series are factor values, target positions, orders, fills, NAV, simple
returns, risk exposures and cost ledger. They stay outside RPC and PostgreSQL.
Row counts are left absent rather than invented from unparsed artifact bytes.

Current-context IDs equal the context object's SHA-256; `latest` aliases are
refused. The six provenance components resolve actual source, configuration,
data, calendar and environment file-set hashes plus the independently validated
operator registry's semantic identity. That last identity is domain-separated
by `loop-core`, not a substitute raw file checksum. The registry's actual file
must also match its canonical bytes. All nine FactorSpec policy references bind
their actual configuration documents.
The configuration fingerprint also pins the backtest engine and version, so a
different catalog cannot silently change the engine of an already queued job.

Only 2005-2006 warmup, 2007-2016 IS and 2017-2020 development-validation roles
are understood here. Warmup cannot register backtest metrics; admission and
perturbation require IS. Both 2021-2024 and 2025-2026-08 remain protected, and
recent data mislabeled as IS is denied by its declared date range. Inspecting
dates/known-at fields inside the data itself belongs to Phase 5 validation:
the current resolver does not promote a declaration to a PIT-quality guarantee.

## Execute, read and export

Host-side backtest submit/acquire/complete, current reads, factor decisions,
perturbation and export invoke the same materialization gate before database
transactions. Each operation retains its own evidence snapshot. The store
rechecks that snapshot under its normal authority, revision, lease and audit
constraints; hashing and numerical work stay outside the transaction.
Infrastructure-failure and budget-exhaustion completion do not require readable
research files: their existing authenticated lease fence may record the outage,
without releasing metrics or adding factor-rejection memory. Acquire and genuine
research decisions still recheck prepared file guards inside the transaction.

Configure the real worker with `PythonPerturber::verified` and a `ResearchBuild`
whose digests match the captured context. An unattested worker is refused by
the concrete resolver. Python recomputes installed build hashes before and after
each transition; drift emits no proposal and commits no optimizer state.
The default-deny resolver remains the executable's production default. Loading
this internal resolver does not open role RPCs or grant filesystem capabilities.

`TrustedManifests::write_current` calls `BacktestRepository::export_current`,
then independently rematerializes and compares the result/current context before
writing `loop.current-backtest-export/v1` JSON to a host-owned async writer. It
contains accepted-at/replay metadata, the context, data grade, return definition
and original result manifest, never inline price series. A sink failure can leave
partial output plus an accepted receipt; discard partial output at the consumer,
then retry with the same semantic key. Retry checks current permissions and files.
The receipt is evidence of accepted metadata release, not exactly-once delivery.

## Bounds and recovery

File loading permits at most 16,384 references, 64 GiB declared bytes, 16 MiB
aggregate metadata and 10 seconds per materializer. Individual metadata is at
most 1 MiB. The shared file cache is bounded to 16,384 entries and 32 MiB metadata;
operations retain their own descriptors, so eviction does not replace evidence.
An overall preparation has a 10-second timeout even when resolving two contexts.
Regular files are opened with no symlink following; inode/device, size, mtime,
ctime and the current directory entry are checked again at each consumption.
These checks cannot defeat a malicious administrator or compromised kernel.

Build capture is limited to 8,192 files, 1 GiB and 60 seconds; verification allows
8 seconds per pass. The verified Python subprocess has a 20-second deadline
(the legacy unattested fixture path remains 10 seconds); the encompassing
perturbation command remains at most 30 seconds. A JSON export is at most 2 MiB
with a 10-second delivery timeout. These are operational fail-closed bounds,
not throughput promises for production datasets.

Restart rematerializes the pinned catalog. Missing or modified original evidence
is a hard failure; a changed resolved current component is `stale`. Neither case
rewrites old metrics or creates a factor rejection. There is no new migration,
table or service. Disable the concrete resolver/export host integration to roll
back, keeping every object, trial, receipt and audit event. Do not remove history
or downgrade the already committed schema. Acceptance evidence is maintained in
`docs/verification/phase-04-research-integrity.md`.

Workspace and CI gates run one Rust test case at a time so unrelated fault tests
do not consume a real worker's verification deadline on small hosts. The process
tests still launch 2/4/8 simultaneous OS writers and execute the full kill/restart
matrix. This scheduling choice does not increase production deadlines or reduce
the writer-concurrency acceptance requirement.
