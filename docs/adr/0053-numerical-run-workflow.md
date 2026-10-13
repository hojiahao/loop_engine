# ADR 0053: Frozen candidate-to-numerical research workflow

Status: accepted for implementation; Phase 11 delivery 2. Completion requires
the installed workflow, negative/recovery checks, documentation, commit, push
and remote gates. This document is not an implementation-completion claim.

## Requirement

Connect the persistent Discovery run to the existing factor evaluator and
portfolio producer. A successful model response currently ends a round without
measuring the candidate. Reuse the numerical jobs, data broker, trial registry
and completion receipts instead of adding another queue or execution engine.

## Decision

Keep frozen v1 discovery-only plans and receipts byte-compatible. Explicit
workflow version 2 selects a v2 run document with an immutable numerical recipe.
Requests must declare the workflow version before executing or mutating a run.
Each round advances Discovery, factor evaluation and portfolio in separate
steps. One request performs at most one expensive stage; the next child is
reserved and created in the same parent transaction. Reservations are never
refunded. Every child must fit the remaining steps and absolute deadline.

The run owner remains an authenticated human. Numerical work uses the separately
registered Research service identity from the frozen plan, after checking its
role, validity, subject and exact run scope. Internal delegation is not a TLS
principal. Discovery receives neither numerical nor protected-data authority.

A closed, content-addressed recipe binds the actual IS data context, execution
tape, nine numerical policies, fixed direction and seed, and bounded hypothesis,
required observations and falsification criteria. These narrative fields are
administrator declarations, not claimed model reasoning or measured results.
Canonical AST evidence comes from the completed model receipt. Derivation uses
the current canonical factor implementation and actual-file resolvers. No auto
direction selection, per-round seed change or unregistered ticker/data input is
allowed. Generated artifacts use existing immutable publication and guards.

## Persistence and failure semantics

Migration 0016 adds append-only run transitions, one per parent revision. A
transition binds the source record, exact child specification digest and full
artifact references including sizes. Jobs remain the sole execution state;
the parent is only the current projection. Final filtered rounds retain a real
source job without fabricating numerical jobs or complete return histories.

Duplicate and previously rejected factors are checked against actual registered
trial evidence inside the transaction. Coverage filtering uses the completed
evaluation trial. Infrastructure failures, expiry and budget exhaustion remain
distinct from empirical rejection. Portfolio completion is a measured result,
not factor admission. Checker, economic gates, independent reconciliation and
licensed-data eligibility remain separately required.

Whole-population trial statistics retain every registered numerical attempt.
Reading managed-run history verifies the committed transition and exact job
specification and authorizes each run. Missing, inaccessible or corrupt members
deny the whole population; they are never silently filtered out.

## Verification and rollback

Require cross-language v1/v2 contracts, actual installed CLI generation/evaluation/
portfolio execution, duplicate and coverage outcomes, budget/deadline exhaustion,
cross-role and holdout denial, corrupt artifact rejection, idempotent crash
recovery and independent-process revision races. Reuse numerical goldens and
global-trial registration tests. No live supplier or licensed-data claim follows
from synthetic acceptance.

Disable v2 plan registration to stop new writers while retaining owner-scoped
status, immutable transitions, reservations and receipts. Do not down-migrate
or rewrite v1 evidence. Older writers are fenced for managed jobs by the schema;
restore the compatible reader before resuming v2 work. Production deployment and
holdout unlocking are separate decisions.
