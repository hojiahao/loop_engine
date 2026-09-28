# Authorized global statistical reports

This development-only workflow implements ADR 0036 using existing PostgreSQL
jobs, leases, receipts and audit events. The caller cannot select a profitable
trial subset or supply computed metrics. Numerical work remains in Python.

## Frozen deployment

Enable this optional block beside the existing portfolio configuration:

```json
"statistics": {"output_store": "/srv/loop-engine/statistics"}
```

Administratively create this private, runtime-owned directory before startup.
It must be separate from evidence, protected data, views and other worker roots.
The existing exact `JobPin` must identify a `JOB_KIND_REPORT` with
`ArtifactJobInput`. Its input is a CAS JSON artifact whose schema name is
`loop.global_statistics_policy`, version 1. The policy reference must bind the
same ID, revision and SHA-256. Example bytes, in declared field order:

```json
{"schema":"loop.global-statistics-policy/v1","policy_id":"policy.global","revision":"1","scope":"all-database-development-trials","minimum_sessions":16,"hac_lags":1,"pbo_blocks":4}
```

The research identity needs access to every development trial in the database.
Discovery, Provider and holdout identities cannot execute this operation.
Operators may read current reports but cannot run the numerical worker.

## Authenticated workflow

Acquire the report job's ordinary lease, then call internal mTLS RPC
`loop.jobs.v1.JobService/ExecuteStatistics` with `context`, `job_id`, `lease_id`
and `expected_revision`. Generated clients exist in all three languages; a
dedicated `loopctl` command is not implemented yet.

The server captures every development factor-evaluation/backtest job, revision,
state and acquired attempt. It resolves successful portfolios to their registered
factor evaluations and reconstructs original numerical evidence. The worker
receives verified references and broker-selected development views. Reports do
not count as additional strategy trials.

Portfolios completed before later trials retain their original immutable search
commitments. The global worker verifies those historical identities against the
current registry and reconstructs original numerical returns, then recalculates
global statistics over the new population. It never rewrites or releases an old
source's search-adjusted statistics as current. Ordinary primary reads still
reject stale trial counts.

The completion transaction rechecks the whole population, registered sources,
file guards, authority and live lease. Result, command receipt and audit append
commit together. Generic `CompleteJob` cannot import an administrative report as
authorized proof.

`ReadStatistics` accepts only the report job ID and reconstructs all evidence.
It returns an artifact reference; return-matrix rows never cross RPC. Completed
command retries verify original files without repairing them. New trials,
changed state, build drift or corruption deny a current conclusion while leaving
historical results and audits intact.

## Interpretation

Strategies need exactly the same ordered return dates and comparable
data/build/calendar context. No date intersection or missing-return filling is
performed. Duplicate frozen configurations share a column only when their
returns are identical; all jobs and attempts remain separately counted.

Pending/unsuccessful work, incomplete retry history, missing continuations,
incompatible samples and fewer than two distinct strategies yield explicit
unavailable diagnostics. Infrastructure failures are never invented as trading
losses. A job may successfully register such a diagnostic without having a
complete matrix or an admissible factor.

The worker reuses HAC, conservative global BY, DSR and exhaustive equal-block
CSCV kernels. DSR retains the all-distinct-strategies-independent sensitivity
assumption; it does not estimate independent search count. CSCV is an internal
selection diagnostic without refitting or holdout access. Supported profiles
remain synthetic/public-development and `production_eligible` is always false.

Limits: 4096 trial jobs, 65536 attempts, 64 successful portfolio sources, 8192
sessions, 100000 return cells, 1 MiB worker requests and 180 seconds additionally
bounded by the live lease. Overflow is rejected, never silently truncated.

## Acceptance and rollback

Tests cover actual opposing-strategy returns, immutable replay, complete
population, duplicates, failed/retried work, incompatible axes, corrupt sources,
authenticated registration, imported-success denial and expired leases. Run
native numerical tests separately from Rust compilation on small hosts.

Disable the optional deployment or revert the implementation commit to stop new
writers. Retain policies, reports, primary results and audits. No database table,
schema migration or destructive down-migration is introduced.
