# ADR 0028: Verified evaluation trials and admission lineage

- Status: Accepted; published and remotely verified
- Date: 2026-09-15
- Owner: hojiahao

## Requirement

Phase 6 unit 3 connects actual numerical evaluation to trial accounting,
deterministic failure memory, duplicate filtering and the existing shared
admission/readmission command. Portfolio construction remains Phase 7.

## Decision

Keep a successful numerical computation distinct from a passing research gate.
The installed worker still produces values and coverage. The runtime resolves
the FactorSpec's actual evaluation policy and applies its frozen integer
`minimum_coverage_bps`. A successful computation atomically records one immutable
evaluation projection with the job completion, trial binding, receipt and audit.
Its disposition is `ready_for_backtest` or `insufficient_coverage`; neither is
factor admission or portfolio performance. Worker, data and infrastructure
errors cannot create this projection or empirical rejection memory.

Add one PostgreSQL projection, indexed by a versioned context digest over the
canonical FactorSpec, ordered snapshots, six provenance components and seed.
Job/run IDs and budgets are excluded. Submission and lease acquisition check
completed evaluations in the same ledger transaction. A prior coverage failure
returns `PreviouslyRejected`; a prior passing evaluation returns
`AlreadyEvaluated`. Already executing jobs may finish and remain separate trials;
this is completed-evidence deduplication, not a claim of global in-flight
uniqueness. All accepted attempts remain countable, including failures/retries.

Extend trial reads with verified evaluation evidence and disposition. Reads
recheck the immutable job/result projection; numerical replay and file-backed
admission additionally resolve actual artifact bytes. This metadata read alone
does not certify current source availability or authorize data access.

File-backed admission reviews use version 2 and name the completed evaluation
job and manifest. Resolve the actual manifest bytes before locking the ledger;
the shared admission command checks the registered evaluation, trial, coverage,
frozen factor/data/seed, values artifact and policy. Backtest configuration can
differ from evaluation configuration because it names a different engine;
source, environment, operators, data and calendar must still match. The factor
identity binds all nine frozen policies. Version-1 review artifacts remain
readable historical evidence, but the deployed manifest resolver denies new
admission from them. Existing explicitly fabricated policy test doubles exercise
legacy decision accounting only and do not establish numerical provenance.

Version-2 preparation verifies the actual worker's full native environment.
Reuse the numerical resolver's 30-second verification budget for this path;
the legacy 10-second budget cannot accommodate its cold byte verification on
the development host. Overall preparation, caller deadlines, file/object limits
and corruption checks remain bounded and fail closed. No work moves into the
database transaction, and no cached trust replaces actual-file guards.

No new service, RPC, numerical dependency or alternate readmission handler is
introduced. Passing coverage waits for a registered primary IS backtest and
completed review. Force cannot waive numerical evidence or coverage. Protected
samples and paid data remain unavailable to this development workflow.

## Verification and recovery

Exercise installed-worker completion through TLS/PostgreSQL, trial visibility,
coverage boundaries, restart, duplicate/failure filtering, changed context,
infrastructure separation, corruption, shared admission and negative linkage.
Exercise independent 2/4/8 processes and kill/restart around the new transactional
projection; retain existing numerical/no-lookahead and language contract gates.
Each independent process verifies its real files before the shared write
barrier opens. Preparation is sequential to avoid testing concurrent cold-build
hash throughput; the 2/4/8 database commits remain concurrent. The fixture uses
the existing allowed five-minute lease, without changing production limits.

Migration 0010 adds the projection without rewriting completed historical jobs.
It marks pre-migration successful evaluations with immutable baseline rows
without numerical evidence; they remain trials but cannot supply new admission
or duplicate evidence until a fresh authorized evaluation completes. New writers
must use the atomic projection path. Rollback stops new writers and retains the
schema, artifacts, receipts and audit; use a compatible reader, not a destructive
down-migration. Do not claim closure until commit, push and exact-commit CI pass.

Implementation `b792601` is pushed. All seven jobs in CI run `35058329373`
pass, including Rust, unified workspace and clean DaoCloud container acceptance.
This closes Phase 6 unit 3 and the factor-engine phase; it does not implement
portfolio accounting, deploy migration 0010 to production or unlock holdouts.
