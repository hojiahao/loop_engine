# Authorized Factor Evaluation

This development-only path computes raw factor values and exact coverage. It
does not construct portfolios, admit factors, unlock holdouts, or establish
point-in-time market-data quality. ADR 0020 and the Phase 4 verification record
track acceptance; the presence of this document is not a release claim.

## Deployment Boundary

The optional mTLS runtime adds `JobService.EvaluateFactor`. It accepts only a
command context, an existing job ID, its lease ID and expected revision. No
executable, model response, file path, factor replacement or output artifact can
be supplied in that request. A deployment-pinned research identity, run and exact
job envelope remain required. Discovery, provider, human-operator and protected
worker identities cannot call this development operation.

The startup configuration's optional `evaluation` section selects:

```json
{
  "evaluation": {
    "python": "/opt/loop_engine/.venv/bin/python",
    "output_store": "/var/lib/loop-engine/factor-output",
    "contexts": [
      {
        "job_id": "job.example",
        "context": {
          "sha256": "sha256:<actual-context-digest>",
          "byte_size": 1234
        }
      }
    ]
  }
}
```

This is a configuration fragment, not a complete deployment or valid example
digest. Contexts and all their referenced files must exist in the separately
configured development CAS. The output directory must already exist, be
runtime-owned with mode 0700, and not overlap development, protected or view
namespaces. Without the section, evaluation denies. The fixed installed Python
module runs with an empty inherited environment, isolated imports and bounded
stdin/stdout. It is trusted numerical service code, not a generic process sandbox
or an Agent-controlled launcher. The existing separate worker/container trust
zones remain required when deploying model-facing processes.

Capture the actual installed numerical build using the existing command:

```bash
loop-research build-manifests --profile evaluation --store /absolute/development-cas
```

The evaluation profile adds panel-validation, pandas, calendar and timezone
dependencies to the source/environment fingerprints. Verification is bounded by
8,192 files and 1 GiB, with a 20-second read deadline and a 60-second publication
deadline. The older perturbation profile retains its 8-second read deadline.
These are byte-backed build identities, not kernel or whole-host attestation.
The Rust resolver verifies the frozen evaluation context within 30 seconds;
other manifest paths retain their 10-second bound. This larger context contains
the actual dataframe/calendar native dependencies, not just configuration JSON.

## Frozen Inputs

The durable factor-evaluation input must carry all six provenance components
and its deterministic seed. Historical envelopes with neither field stay
readable but cannot execute. A partial pair is invalid. The pinned context must
match the durable input, actual canonical operator registry, all nine policy
references, source/environment files, data and calendar. Its configuration names
`factor-evaluator.2`; this identifies the raw-value implementation, not a completed
primary portfolio backtester.

Each current development snapshot contains exactly two artifacts:

- `loop.factor_panel`, version 1, `application/json`.
- `loop.factor_panel_values`, version 1, `text/csv`.

The canonical `loop.factor-panel/v1` JSON field order is `schema`, `quality`,
`sessions`, `securities`, `fields`, `decision_times_ms`, `evaluation_start`,
`values`. The final value is an exact CAS digest/byte-size reference. Sessions
must equal the frozen calendar and contain the complete requested XNYS interval;
securities and fields are sorted and unique. The CSV header is:

```text
session,security_id,eligible,known_at_ms,<sorted semantic field names>
```

Rows form the exact session/security Cartesian grid. Eligibility is explicit
`0` or `1`; an empty value means missing, never zero. Observed values must be
finite binary64 and have a visibility timestamp no later than that session's
declared decision time. Decisions must follow the actual exchange close,
including half-days, and remain on the same New York session date. No implicit
join, row fill, trimming, current-ticker lookup or forward fill occurs.

Only `synthetic` and `public_development` quality and the existing IS/development
date boundaries are accepted. Warmup contributes only to causal calculations;
returned values and coverage start at the explicit evaluation boundary.
Validation of these declarations cannot prove a supplier's historical universe
or visibility timestamps. Phase 5 supplies that independent data-quality work.

## Execution And Recovery

The fourteen operator families use semantic version 2. Rolling calls explicitly
specify `(series, width, minimum_valid)`; lag calls use `(series, width)`.
Skew uses adjusted Fisher-Pearson moments and actual valid count; sample standard
deviation uses ddof=1. Constant skew/zscore remain missing. `rank_ts` uses stable
last ties and a midpoint for constant windows; `rank_cs` uses average valid
ranks. Arithmetic is binary and never normalized using floating-point
associativity. Factor direction is preserved and is not reapplied to raw values.

The runtime verifies a live lease before preparing a readonly data view, runs
the fixed worker, rechecks source/environment and data, resolves the actual
output bytes and then uses the existing fenced completion transaction. The
ordinary completion RPC cannot register unchecked factor success. Output files
alone are not completion authority.

The worker is limited to the smaller of 60 seconds and the current lease's
remaining lifetime, which is already capped by the original job deadline. Data
preparation does not reset that deadline. The runtime RPC has a 90-second outer
timeout; timeout or request cancellation drops and kills the subprocess. These
are ceilings, not throughput estimates. Full-market runtime must be measured
with the licensed dataset, universe and expression depth of the actual study.

Successful output contains raw-value CSV plus a canonical result manifest,
including exact factor/lease identities, six fingerprints, seed, quality,
sample bounds, coverage and work counts. Frozen civil-date sample bounds are
preserved even when the first/last boundary is a nontrading day. The raw values
are deterministic; independently created completion timestamps are metadata.
Completed retries resolve the stored output and immutable receipt instead of
recomputing or appending a second completion audit.

`GetJob` exposes the historical job envelope and artifact references. It does
not certify that historical values remain current under a new research context.
The checked evaluation replay verifies the pinned inputs, installed worker build
and stored outputs again; no generic current-value exporter is introduced here.

On failure or cancellation, no successful completion is authorized. Immutable
unreferenced output can remain after a crash; do not treat it as admitted work.
Disable the optional evaluator to roll back and retain all accepted artifacts,
jobs, receipts and audit history. No new schema or destructive down-migration is
required. Real holdout execution and production market-data retests remain off.
