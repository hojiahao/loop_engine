# Phase 10 unit 5: CLI and deployment recovery

Status: complete. Implementation and local operational acceptance pass;
task `b692e30` and bounded image-preparation correction `5a25008` are published.
Exact-commit CI `37733309351` passes all seven jobs, including the full unified
workspace and clean DaoCloud container. Design: ADR 0050. Operations:
`docs/development/discovery-cli.md` and `docs/development/production-cutover.md`.

## Remote prerequisite

CI prerequisite: lifecycle commit `12768ff` passed six jobs in run `36689533660`.
The unified job reached its 75-minute limit after the full Rust suite and 758
Provider tests passed, while Python research was at 96% without failures.
Its 337 loopd library cases took 3,351.45 seconds; test-profile compilation took
only 1 minute 42 seconds. Keep the serial real-process checks and increase this
full clean-workspace job's bounded budget to 120 minutes. The descendant commit
must pass the entire job before either unit's remote gate is closed.

For `b692e30`, Rust, Python, TypeScript, legacy and clean DaoCloud-container jobs
all passed. The unified job completed `just check` and the full `just test`, then
failed in `just test-isolation` before Provider startup: DaoCloud returned a TLS
handshake timeout while Docker implicitly pulled the pinned Node image.

Prepare that exact digest in a separate bounded hook: use the local exact
reference when cached, otherwise try the same DaoCloud pull at most three times
with 60-second deadlines and 1/2-second backoff. Verify the local image after a
successful pull. The hook has a 195-second ceiling; the existing behavioral test
retains 180 seconds. Compose up and client run both use `--pull never`, and every
rendered service must match the same pinned digest. No registry fallback,
tag-only substitution, workflow retry or model-call retry is introduced.
Actual local Provider/container isolation passes in 23.57 seconds after this
correction; syntax, formatting and naming checks pass. Rollback restores only
the test setup; deployed runtime, database and research evidence are unaffected.

## Local evidence

The CLI unit suite passes all 56 cases. All 15 PostgreSQL observation cases pass,
including owner/run checks, foreign cursors, page/index bounds and checksum/link
corruption. Protocol regressions pass 129 TypeScript and 326 Python cases; wire
regeneration and role/protocol boundary checks pass. `loopctl doctor --json`
retains the existing ready envelope.

Initial process acceptance found an incorrect test assumption: cancelling the
caller after outbound dispatch does not guarantee a completed Provider receipt.
The interrupted transport keeps an ambiguous claim. The corrected cases assert
retained reservation, bounded lookup failure, no candidate and one supplier
call, while successful committed work is checked separately across restart.
All nine installed CLI process cases and the two adjacent tool recovery cases
now pass. A serial batch initially hit fixture startup/migration/lease deadlines
while the host had substantial I/O and swap pressure. Each of the five affected
cases subsequently passed in isolated runs (7.47 seconds, 5.76 seconds, and a
three-case batch of 17.28 seconds), without changing production timeout bounds.
The successful six cases included killed-process recovery and stop-only control.
This is aggregated local evidence, not a claim that the first batch passed.

All 10 Rust Discovery protocol-boundary cases pass. Complete workspace checks,
including Clippy with `-D warnings`, passed; final formatting, function-name and
diff checks pass after the documentation-only service comment correction.

## Authorized first production install

On 2026-10-08, installed `loopd` and `loopctl` on the explicitly authorized host
`117.50.81.155`. This was its first Loop application deployment. No existing Loop
service or research workload was replaced. The database had no jobs, command
receipts or audit events; all migration-6/7/9 historical blockers were zero.

- Preserved a private custom-format database backup at
  `/var/lib/postgresql/loop-backups/20261008-first-install/loop_engine.dump`;
  `pg_restore --list` passed. SHA-256:
  `2517632f3e1d88e49964e7cd92df1d685c29f52e91c60e2dcdd8eedaac49fa76`.
  A subsequent isolated restore into `loop_engine_restore_20261008` also passed:
  migrations 1–5 all succeeded, zero jobs/receipts/events matched the source, and
  the ledger identity remained `ledger.loopd`. The temporary database had public
  connection permission revoked and was deleted after verification. Production
  retained all 14 migrations, zero jobs, an active service and readiness 204.
- Applied the reviewed production bundle transactionally; all 14 migration
  receipts are successful and the installed binary verified exact checksums.
- Runtime connections use PostgreSQL TLS. Runtime has no schema CREATE, owner
  membership, DELETE/TRUNCATE or UPDATE outside the mutable allowlist; evidence
  protection triggers remain enabled.
- Release: `/srv/loop/releases/20261008-12768ff-worktree`, selected by
  `/srv/loop/current`. Its manifest records the base commit, worktree source
  snapshot, binary build times/digests and protocol descriptor. These are
  unoptimized development-profile alpha binaries, not an exact-commit optimized
  production release. The release name deliberately identifies that distinction.
- `loopd.service` runs as the dedicated non-login `loopd` account and is enabled
  at boot. HTTP is loopback `18080`; mTLS is loopback `18443`. Port 8080 belongs to
  another application and was preserved.
- Health returns 200, database readiness returns 204, including after an actual
  systemd restart; exit status is zero and automatic restart count is zero.
- TLS 1.3 with client certificate and `loopd.internal` hostname verification
  passes. Actual CLI status/execute probes against an absent job return exit 3:
  unknown jobs are intentionally hidden as authorization denial, not not-found.
  No synthetic research job was inserted into production to claim acceptance.
- All executors are omitted from the private runtime configuration. Provider,
  numerical research and autonomous discovery execution remain disabled. No
  model API call, protected-sample access or paid-data retrieval occurred.

Rollback stops the service/new writers and retains the upgraded database,
backup and evidence. There was no previous application release to restore;
future replacements must understand all 14 migrations. The application unit is
versioned at `infra/systemd/loopd.service`; private credentials and certificates
are never part of the release or Git history.

On 2026-10-09, reverified the same deployment on `117.50.81.155`: both installed
binary SHA-256 digests match the original release manifest; the service remains
active and enabled, with zero automatic restarts and main exit status zero.
The actual health endpoints are `/healthz` (200) and `/readyz` (204) on loopback
18080; `/health` and `/ready` are not routes. The installed binary's database
check succeeds; migrations 1–14 are successful, runtime sessions use TLS 1.3,
forbidden table privileges and disabled protection-trigger counts are zero.
Jobs, command receipts and audit events remain empty. Client-certificate TLS 1.3
verification on loopback 18443 succeeds, and the installed CLI denies the absent
probe job with exit 3. Every optional executor remains disabled.

Commit `5a25008` changes only test setup and documentation relative to `b692e30`;
it requires no production binary replacement or database migration. This
reverification preserves the original release identity and development-profile
limitation rather than relabeling the binaries as an optimized release. No
temporary deployment files, new research jobs or paid supplier calls were made.

## Required evidence

- Installed `loopctl` processes use mTLS against actual Discovery services and
  the compiled Provider; local synthetic supplier, real PostgreSQL, no paid keys.
- Start/status/execute/events, pause/resume, cancellation and evidence-only
  reconciliation preserve job identity, reservations and supplier call count.
- Actual `loopd` binary stops on SIGTERM, restarts with the same namespace and
  resumes immutable requests without duplicate dispatch.
- Stop-only deployment observes state/events, denies execution and permits stop
  controls even with unavailable plan files.
- Replayed submission, CAS/key conflict, wrong identity/certificate, bounded
  deadlines, SIGINT, response JSON/exit codes and redacted errors.
- Scoped event pagination, foreign cursor denial, checksum/link corruption,
  unknown operation denial and no raw payload or protected-type reachability.
- Existing descriptors, generated bindings, numerical goldens, workspace checks
  and exact-commit CI remain required; prior unit CI must pass as well.

Disposable fixtures, copied binaries and database containers are removed after
tests. Toolchains, licensed data, retained research and unrelated project files
are not test cleanup targets.
