# ADR 0018: Trusted research manifests and file-backed evidence

- Status: Implemented; local gates passed, publication pending
- Date: 2026-09-11
- Owner: hojiahao

## Requirement

Phase 4 delivery unit 3 must replace fixture hash equality with verification of
actual immutable files. Calculation, result registration, current reads and
audited exports must resolve the same frozen inputs. This does not introduce a
market-data feed, a primary backtester or a transport authentication mechanism.

## Decision

Use one local content-addressed object store and a service-owned, explicitly
pinned catalog. Caller-provided URIs are never filesystem paths or authority.
The catalog binds jobs and content-addressed current contexts to immutable manifests. It
does not accept moving aliases or automatically discover trust from a directory.
Canonical AST/FactorSpec parsing reuses `loop-core`; a deployment supplies the
validated operator registry. Strict versioned JSON binds the sample, seed,
policies, provenance, result artifacts and review evidence without self-hashes.
The configuration fingerprint includes backtest engine/version, preventing a
replacement deployment catalog from changing a queued job's execution choice.

Materialize and checksum files asynchronously before acquiring a database
transaction. Retain opened regular-file identities and verify their inode,
size, modification and change times at each synchronous policy use. Check the
current directory entry as well, so replacing a pathname cannot keep an old
descriptor current. Recheck after calculation and before releasing an export.
No large file scan, network request or numerical work runs under a ledger lock.
Each operation owns its prepared evidence snapshot. Only the bounded verified
file cache is shared; another request cannot replace its result/current context.

The installed Python perturbation worker captures actual research/protocol,
NumPy/Protobuf, bundled native-library and interpreter bytes. It verifies the
pinned source/environment manifests before and after a numerical transition.
The source/environment contract is specific to this worker, not a hash of every
OS library or proof of a hermetic host. The primary backtester and other future
workers must explicitly extend and bind their own execution dependencies.

Audited JSON export uses the existing metadata-release command, rematerializes
the files and rechecks result equality before writing. Delivery is bounded and
cancellable. A partial sink failure leaves the committed acceptance receipt;
retry reauthorizes, revalidates freshness and does not append a second acceptance.
An acceptance event never claims a file reached its final destination.

Resolve paths relative to a pinned directory descriptor with no symlink
following; reject traversal, special files, oversized metadata and bounded-scan
exhaustion. The safe `rustix` filesystem API supplies `openat`/`NOFOLLOW` without
handwritten unsafe code. This dependency is already present transitively; using
it directly avoids a check-then-open symlink race or platform magic constants.

Checksums and file-version guards detect accidental changes and unprivileged
replacement. They are not an attestation against a compromised kernel or an
administrator rewriting the deployment and all evidence. Task 4 supplies the
runtime identities, read-only mounts and capability isolation; unresolved or
protected references continue to deny by default.
Only synthetic and public-development data grades are accepted. Calendar and
dataset manifests prove the producer's frozen declarations and referenced bytes,
not complete session coverage, point-in-time correctness or profitability.
Phase 5 must independently establish those data properties. Production role
submission, transport credentials and protected-data execution remain disabled.

## Verification And Recovery

Exercise actual file bytes and PostgreSQL command paths, not an in-memory result
resolver. Include missing files, changed bytes, path replacement, symlinks,
special files, malformed/cross-bound manifests, stale components, restart,
worker drift and failed exports. Synthetic results remain labeled synthetic.
No fixture can establish production data quality or numerical profitability.
Independent 2/4/8-process export races and kills after receipt insertion,
before commit and after commit must also use the actual file-backed resolver.
Usage, limits and byte formats: `docs/development/research-manifests.md`.

Final local `just check/test/build/doctor` pass. Detailed counts, real-process
coverage and the resource-contention rerun are recorded in
`docs/verification/phase-04-research-integrity.md`. No production endpoint is
enabled; publication and remote CI remain required before task closure.

This task adds no database migration, table or service. Disable the concrete
resolver/export writer to roll back; retain all immutable artifacts, receipts,
trials and audit history. Local and remote gates, commit and push are required
before this task may be marked complete.
