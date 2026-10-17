# ADR 0054: Verified result feedback and leased numerical perturbation

Status: accepted for implementation; Phase 11 delivery 3a. The preceding
candidate-to-numerical workflow passed both seven-job CI gates and merged in
PR #4. This decision does not mark the adaptive workflow implemented or verified.

## Requirement

Use earlier, verified IS results to inform the next frozen model request and
reuse the installed deterministic window perturbator in the same evaluation and
portfolio workflow. Persist Sharpe observations, random state, proposed IDs and
failure evidence across restarts. Keep infrastructure failure, unavailable
statistics and empirical rejection distinct. This unit adds neither playbook
promotion, a semantic checker, formal admission nor background scheduling.

## Ownership and compatibility

Explicit workflow version 3 selects an immutable feedback policy in the frozen
run specification. Versions 0 and 2 retain their wire bytes, budgets, receipts,
closed input validation and projections. All model routing remains in the
existing Provider/Harness path; the Research identity executes perturbation.
No RPC accepts a replacement policy, arbitrary work/state, model invocation,
source selection or protected capability.

The policy fixes a finite proposal schedule, single-window selection rules,
ordered eligible windows, random seed and bounds for structured feedback.
Feedback selects at most 16 earlier registered observations and encodes at most
16 KiB; the full invocation must also fit the existing message and token
allowances. Source IDs, full artifact references, applicability and unavailable
values remain structured evidence. No auxiliary LLM distillation is added.
The first round uses Discovery. Later rounds may replace Discovery with one
Perturbation child, then use the same Evaluation and Portfolio path. A round
still has at most three children; existing 64-round/history bounds remain.
The first eligible model candidate establishes a fixed perturbation family.
Later model candidates may be independent of that family and still contribute
verified feedback; family mismatch is not corruption or empirical rejection.
If the scheduled perturbation lacks a family with an eligible observation, freeze
a Discovery fallback before creating the child, without consuming a random draw.
Successful perturbation exhaustion terminates as `SearchExhausted`, preserving
the actual completed-round count rather than claiming all planned rounds ran.

## One execution system

Append a real Perturbation JobKind and internal typed input to the existing job
contract. Its input pins the frozen policy, development dataset, parent binding,
family, immutable work and installed worker identity. Its allowance has one
step, no model tokens and no model cost. Reserve it before insertion using the
existing parent transaction. It is not a Discovery response or factor trial.
Only an actual Evaluation submission increments the numerical trial population.

Use the existing job lease, deadline, cancellation, terminal completion and
recovery controls. Acquire the lease before launching Python; competing requests
must not start duplicate workers. Run Python outside a database transaction and
terminate its child on cancellation, lost authority or deadline. Retain its
bounded input/output and wall time; do not describe these as an account-wide
CPU or cost quota. No new queue, service or state table is needed.

The successful job output is an immutable, versioned receipt binding exact work,
before/after state, candidate or exhaustion, algorithm and build. A private
completion proof verifies these bytes, lease, job revision, source authority and
final file guards. Generic Complete without this proof must deny the new kind.
An exhausted search is a successful selection result, not a fabricated factor
rejection, Sharpe value or infrastructure fault.

## State and evidence

Keep bounded, versioned adaptive state inside the existing append-only
RunBinding. Reuse PerturbationWork/State/Step, the Python algorithm and pure
transition validation. Leave the legacy global perturbation_states API and its
fixed-context semantics unchanged. Additive migrations extend kind/version/stage
constraints and old-writer fences; never edit published migrations.

The child result commits before parent advancement. The parent transaction then
verifies the successful child, applies exactly one RNG/history/proposed-ID
change, updates feedback and reserves/inserts the next child or records a
pre-screen result. State, parent revision, child/trial, receipt and audit remain
atomic. A crash between child completion and parent commit reuses the original
result; neither worker execution nor random draws are repeated. Reconstruction
checks each adaptive delta against its exact source receipt, not only the latest
state digest. Missing/corrupt evidence denies advancement without repair.

Families are run-local and bind the frozen recipe, IS context, direction,
policies, engine/build, seeds and canonical AST with exactly one allowed window
position masked. Every observation retains its own actual candidate provenance;
different portfolio configuration hashes must not be relabeled as identical.
Before use, resolve the real registered result and files, require the same run
and an earlier committed round, and recheck all authority and freshness guards.
Holdout outcomes are forbidden even as summaries. Infrastructure failures and
missing Sharpe values never become zero scores or empirical failure memory.
Every verified successful same-family factor is also excluded from subsequent
perturbation proposals, including sources not yet consumed by the incremental
Sharpe observation state. A bounded `observed_factor_ids` work field carries
this source-derived set separately from failures and proposal history. Preparation,
receipt recovery and transition replay require the exact frozen-family-ordered
set; the legacy global perturbation writer leaves this additive field empty.

The installed portfolio statistic is sharpe_annualized/ratio/daily-statistics.1.
Adapt it only after checking the underlying cost-adjusted daily return basis,
zero risk-free rate, sample standard deviation and sqrt(252) annualization.
The legacy net_sharpe label alone is not a semantic equivalence check. Preserve
unavailable statistics and their reasons; only eligible finite values can enter
the existing numerical observation type.

Freeze the complete next model invocation and feedback-source manifest before
any model call. Add a private server-derived context proof to the v3 Discovery
path while retaining exact static-template matching for old plans. Recovery
reads those immutable bytes and the committed model receipt; it does not build
a new prompt or automatically resend an ambiguous paid call. Model output itself
is not promised to be deterministic.

## Acceptance and rollback

Require an installed three-round fixture: model candidate and real IS portfolio,
leased perturbation and the same numerical path, then a model invocation with
both eligible prior results. Cover unavailable Sharpe, insufficient coverage,
duplicate/failed candidates, exhaustion, budget limits, stale/corrupt/cross-run
and protected evidence, forged completion, expired leases and cancellation.
Two successful model proposals covering a two-window family must make the next
perturbation exhaust without a duplicate evaluation or a random draw.
Race 2/4/8 independent clients and count actual worker launches. Kill/restart
before/after child completion and parent commit must preserve one result, one
RNG application, immutable receipts and exact trial/accounting counts.

Preserve three-language old/new contracts, numerical goldens, default thread
stack, full workspace and clean-container gates. Complete the task's code,
tests, documentation, Chinese commit, push and remote verification before
starting 11.3b. Offline acceptance does not establish live-model benefit or
production data quality.

Disable registration/advancement of v3 plans to stop new adaptive work. Retain
compatible readers, jobs, state, source objects and audit; keep v0/v2 behavior
available. Do not down-migrate, erase failed evidence, reset RNG, refund ambiguous
model charges or recreate missing committed receipts during rollback.
