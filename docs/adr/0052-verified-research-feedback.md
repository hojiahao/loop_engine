# ADR 0052: Verified research feedback and bounded experiment scheduling

- Status: accepted execution-plan amendment; implementation remains pending
- Date: 2026-10-12
- Owner: hojiahao
- Extends: ADR 0016, 0017, 0036, 0048 and 0051

## Requirement and evidence

The owner approved incorporating the applicable Auto-RecSys lessons into the
remaining Loop Runtime work. Improve operational reliability and the quality of
research feedback without replacing the existing stack or weakening statistical
and authorization boundaries. The paper's observational reliability results do
not establish investment performance or a causal benefit for this project.

Reference: [Auto-RecSys, sections 4–7 and 9](https://arxiv.org/abs/2609.10922).
Its unvalidated playbook updates and broad autonomous workflow edits are not
adopted as permissions to change evaluation or production code.

## Decision

Keep Rust orchestration, TypeScript Provider transports, Python numerical work,
PostgreSQL metadata and immutable artifacts. Reuse their current commands,
receipts, trial ledger and access checks. No new agent framework, memory service,
vector database, general code-execution tool or queue is required by this ADR.

Complete the candidate-to-numerical workflow before enabling adaptive feedback.
Each proposal needs a bounded, attributable hypothesis, required observations,
falsification criteria and canonical AST. Narrative fields are claims, not
authority, data-quality evidence or a way to replace frozen policies. Bind the
resulting FactorSpec and jobs to the source proposal. Research execution uses
its separately registered identity, never a relabeled Discovery principal.
Use an opt-in versioned workflow; keep existing frozen discovery-only plans and
receipts readable and unchanged. ADR 0051's same-run reservations must cover
every newly authorized child before its creation.

Maintain two different feedback categories:

- Operational lessons bind a failure condition, verified root cause/remedy,
  affected component versions and successful replay evidence. Infrastructure
  failures, unconfirmed diagnoses and ambiguous model charges are not empirical
  factor rejections. A proposed remedy cannot install dependencies or edit code.
- Research lessons bind the hypothesis, canonical factor, actual registered
  evaluation/portfolio evidence, sample use, cost/universe policies, uncertainty
  and observed conclusion. Record negative and unavailable evidence explicitly.
  A lesson applies only to its validated context; it is not a universal blacklist.

Store full eligible, redacted observable tool/result evidence as immutable
artifacts. Do not request hidden model reasoning or inject whole transcripts by
default. The model sees a bounded selection and derived natural-language text;
structured provenance and source receipts remain the truth. Do not expose
secrets, protected data, confirmation outcomes or protected-run traces to
discovery through either layer, including sanitized summaries.
Distillation and validation model calls use the same durable Harness budget and
invocation ledger; no auxiliary call bypasses accounting. Recovery reuses a
committed candidate receipt. Freeze future request/context bytes, but do not
claim deterministic output from a remote LLM; byte-level candidate reproducibility
applies only to deterministic fixtures/replay and the numerical perturbator.

A playbook revision is an immutable JSON manifest plus optional Markdown, with
source IDs/digests, applicability/expiry conditions and validation evidence.
Distillation proposes a draft only. Promotion checks source authority/freshness,
conflicts, content/tool limits and reproducible development replay. Model review
alone cannot promote a revision. Concurrent promotion uses existing revision
fencing; active runs pin the revision selected at start. No shared mutable
`latest` file changes an in-flight run. A new accepted revision applies to later
runs; deterministic result feedback within one run still follows its frozen
feedback policy and is recorded per round. Invalidated sources block new use
without deleting historical evidence.

Retain preregistered search/evaluation boundaries, direction-selection rules,
costs, universe, numerical semantics, independent reconciliation and admission
checks. Learning cannot grant authority, waive a failed check or reopen a holdout.
Candidate proposal/selection events and pre-screen outcomes remain auditable;
accepted numerical attempts remain in the existing global trial population.
Do not fabricate trials for denied RPCs or claim rejected pre-screens have
complete return histories. Incomplete global statistics remain unavailable.

Scheduling starts with explicit finite work and configured concurrency one.
Background ticks reconstruct bounded context from committed state instead of
keeping an LLM conversation alive while waiting. Retries, leases and ambiguous
invocations retain existing semantics. Enable multiple runs only after a durable
shared step/token/cost/resource reservation scope is implemented and verified;
ADR 0051 budgets cover one run, not an account. Increasing concurrency requires
measured capacity, fairness, backpressure and a bounded-work acceptance case.
Multi-host execution additionally needs authenticated artifact access and real
host-loss/handoff acceptance; PostgreSQL process races alone do not establish it.

## Delivery and verification

The detailed owner-facing execution plan is
[the October plan](../development/execution-plan.md). Each numbered delivery
unit includes behavior, negative tests, documentation, a Chinese commit and
successful remote checks before the next unit. Design inspection while waiting
for a predecessor gate does not count as implementation completion.

Feedback acceptance compares no feedback, fixed playbooks and validated feedback
under the same development tasks, data, model/prompt version, seeds and total
budgets. Predeclare metrics and repetitions; preserve all failures. Measure
known-failure repetition, recovery, duplicate proposals, actual completed
evaluations, resource usage, human attention and independent agreement. Include
all distillation, validation, retrieval, orchestration and replay costs inside
the comparison budget, rather than giving dynamic memory uncounted resources.
Report uncertainty and measure scheduling throughput separately from learning benefit.
Never use a locked confirmation sample to tune memory or claim better returns
from operational reliability alone.
Every comparison round reads only a playbook revision published before that
round starts, without its own or future evaluation outcomes. Separate lesson
generation/validation tasks from effectiveness evaluation, or preregister a
chronological replay. Using only development data does not justify leaking an
evaluation task's answer through its playbook.

Require actual persisted restart, corrupt/stale/misbound evidence rejection,
cross-role/holdout denial, conflicting lesson updates, rollback and 2/4/8-process
races at new transactional boundaries. New numerical paths retain goldens,
cross-language contracts and the global trial/admission regressions.

## Recovery and non-goals

Disable new workflow/feedback/scheduler writers independently while retaining
owner observation, compatible readers, plans, artifacts, budgets and audit.
Revert a playbook by selecting a previously verified immutable revision for a
new run; do not rewrite an active run or erase newer evidence. Schema changes use
new additive migrations and explicit old-writer fences. No destructive
down-migration, refund of ambiguous spend, paid purchase, automatic production
rollout or new holdout unlock is authorized by this plan amendment.
