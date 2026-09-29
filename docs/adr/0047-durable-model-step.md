# ADR 0047: One durable, authorized discovery model step

Status: accepted and implemented; Phase 10 unit 2, local acceptance passed.
Publication and exact-commit remote CI remain delivery gates.

## Problem

Provider receipts alone cannot authorize a research run or reserve its budget.
A crash between outbound generation and result registration must not cause an
automatic second paid request. A model response is a candidate, not admission.

## Decision

Use the existing PostgreSQL job aggregate, lease fencing, command receipts and
audit transaction. One model-step row binds the exact original request and its
Provider-compatible digest to one frozen discovery job. The initial profile has
one step and one candidate, bounded text input, a fixed closed AST output schema,
and a 120-second absolute job deadline. Tool calls and multi-turn execution are
not enabled by this profile.

These ceilings apply to one job, not an account-wide or multi-job run ledger.
Creating another authorized job consumes another explicitly requested job budget;
the outer Loop's cumulative stopping/accounting policy is not delivered here.

An administrator pins immutable plan, invocation, installed operator registry,
development dataset and protocol-selection documents. The plan binds the exact
run, authenticated actor, connector identity and conservative token/USD ceilings.
The research-plan policy and Provider request policy are distinct pinned values;
the latter retains the Provider deployment's own identity.
Protocol selection is administratively pinned, not asserted live negotiation.
Only the development store is reachable; no holdout capability is accepted.

Reservation and initial lease acquisition are atomic. RESERVED -> DISPATCHED
commits before the network call; only its winning CAS may invoke the Provider.
After DISPATCHED, recovery uses authenticated LookupInvocation even if the
Provider reports ABSENT. An uncertain result retains its complete reservation.
Expired leases can be taken over only before the original deadline. Reads of
historical evidence remain possible afterwards; automatic post-deadline recovery
and manual reconciliation belong to the later lifecycle unit.

The Provider connection uses mutual TLS and the same pinned discovery actor.
Rust implements no supplier protocol and holds no supplier API credentials.
Before completion, Rust independently verifies response identity, usage, the
fixed schema and AST semantics, then canonicalizes with the installed registry.
The immutable response and terminal job outcome commit together. Invalid output
is an infrastructure/contract failure, never a quantitative factor rejection.
The narrow Discovery API returns a canonical candidate and reserved ceilings,
never raw prompts, data, lease secrets or an admitted factor.

## Compatibility and rollback

Migration 0011 is additive. Stop old writers before applying it. A trigger fences
generic job updates for tracked model steps; the trusted new transaction enables
its local writer marker. This is an old-writer compatibility guard, not a boundary
against an administrator with arbitrary SQL privileges. Rollback disables the
discovery deployment/endpoint and preserves model_steps, job/audit records and
Provider receipts. Do not drop tables or reclaim ambiguous reservations.
The supported rollback is a feature disable on a migration-aware binary; an
older binary may refuse the newer schema and is not promised write compatibility.

## Acceptance

Require actual PostgreSQL and Provider transport tests, cross-language digest
goldens, 2/4/8 independent-process contention and kill/restart around reserve,
dispatch and completion. Verify actor/model/plan/data drift denial, bounded
budgets, unknown outcomes and no duplicate supplier calls. Formatting or mock
tests alone do not close this unit. Deployment and verification details will be
recorded with the complete task commit.
