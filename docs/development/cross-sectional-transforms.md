# Frozen cross-sectional factor transformations

Version-2 factor panels apply the frozen preprocessing and neutralization
policies during the existing authorized numerical job. They produce factor
values and coverage, not portfolio returns or factor admission. ADR 0027 records
the ownership, compatibility and rollback decisions.

## Build and authorize

Use the same private source and development output stores as described in
`causal-factor-panels.md`. A `loop.panel-build-request/v2` adds a `transform`
object containing the SHA-256 and byte size of a private recipe. The complete
invented example is `fixtures/market/transforms/request.json`; its source,
capture, exposure and recipe files must first be published under their content
digests in the private source store. The integration tests do this explicitly.

The recipe has schema `loop.panel-transform-request/v1`, `preprocess` and
`neutralization` policy documents, and an `exposure_capture` reference or null.
Each policy is the actual `loop.research-policy/v1` document already addressed
by the FactorSpec. Settings are closed, bounded string maps ordered by key.
No expression, direction or policy is selected from the evaluation sample.

Run the existing installed commands against that completed private input:

```bash
./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research panel-build \
  /absolute/private/transform-panel-request.json \
  --sources /absolute/private/source-cache --store /absolute/development-cas

./scripts/uv.sh run --package loop-research --locked --offline --no-sync loop-research panel-validate \
  --receipt sha256:CONSTRUCTION_RECEIPT_DIGEST \
  --sources /absolute/private/source-cache --store /absolute/development-cas
```

Replace the receipt placeholder with the returned `receipt.sha256`. The command
requires no network or new credentials. Pin its dataset/calendar and both exact
policy documents in the deployment's existing research context. Set that
context's engine version to `factor-evaluator.3` and freeze the matching policy
references into the FactorSpec before registering the job. Creating artifacts
does not grant transport identity, job authority, a lease or holdout access.

## Executable policy profile

The preprocessing document must contain exactly these four settings:

```json
{
  "algorithm": "cross-section.1",
  "minimum_observations": "4",
  "standardize": "zscore",
  "winsor_tail_bps": "100"
}
```

`minimum_observations` is an integer from 2 through 10,000. `standardize` is
`none` or `zscore`; z-scores use the sample standard deviation, ddof=1.
`winsor_tail_bps` is an integer from 0 through 2,500 applied symmetrically;
100 clips at the 1st and 99th percentiles using linear interpolation. Integer
strings are canonical: leading zeros, decimals and automatic selection fail.

Neutralization is exactly `{"algorithm":"none"}`, with no exposure capture,
or the following settings with at least one exposure enabled:

```json
{
  "algorithm": "ols.1",
  "beta": "true",
  "industry": "true",
  "log_size": "true"
}
```

Each session independently selects eligible finite factor values with every
required exposure, clips them, applies equal-weight OLS if requested, and then
optionally standardizes the residuals. Industry levels are lexically ordered;
OLS includes an intercept and omits the first industry as the reference level.
Size is the natural logarithm of positive USD market capitalization. Size and
supplied beta columns are centered/scaled within that session. There is no
across-date fit, weighting, forward fill, direction reselection or implicit
fallback to another exposure set.

The solver fixes an SVD relative cutoff of `1e-12` and at most 64 design columns.
It scales the response before solving to avoid avoidable overflow. Sessions
below the configured minimum, or with observations no greater than columns,
produce `insufficient`; a deficient design produces `rank_deficient`. A
residual norm at most `1e-12` times the scaled response norm is treated as zero.
A constant vector requested for standardization produces `constant`. These
outcomes contain missing factor values, not invented zeros or a successful
admission. Without standardization, a fully explained fit can validly return
zero residuals. Solver failures, overflow and resource-budget violations fail
the operation instead of being classified as a poor factor.

The eligible coverage denominator is unchanged when exposures are missing.
Result evidence records valid cells before and after transformation and one
outcome per evaluation session. Raw evaluation and transformations share the
50-million work-unit ceiling and 2-million-cell bound; the existing worker
deadline and lease fencing remain in force.

## Exposure evidence and visibility

The private `loop.exposure-input/v1` capture has development quality, a fixed
`captured_at` equal to the price capture, and at most 10,000 records. Each record
has `security_id`, `session`, `currency: USD`, nullable `industry`, `market_cap`
and `beta`, plus the existing `effective_at`, `known_at`, `ingested_at` and source
evidence fields. Numeric values use bounded exact-decimal strings before the
worker's explicit finite binary64 conversion. Market capitalization is positive;
industry identifiers are bounded ASCII tokens. Duplicate versions, mixed source
series, missing source bytes, quality changes and later ingestion are rejected.

For each security/session, select only the latest revision effective and known
at that session's decision time. Comparisons retain microseconds; CSV knowledge
timestamps round upward to milliseconds. Later observations cannot replace an
earlier session's exposures. There is no carry-forward from another session.
Unavailable components remain empty, including when a later visible revision
explicitly removes an earlier value. The capture's source-file set has its own
512 MiB verification budget; it is separate from the price-source budget.

The derived CSV is an additional `loop.factor_exposures/v1` artifact:

```text
session,security_id,known_at_ms,industry,market_cap,beta
```

The worker requires the complete ordered panel grid, verifies its exact bytes,
checks each visibility timestamp and retains the same file-replacement guards
as the raw panel. Only the derived CSV reaches its read-only view. Private source
responses and the full exposure capture remain outside the broker view.

This unit accepts source-backed data-owner declarations. It does not estimate
beta, infer market capitalization from present-day shares, or certify a vendor's
historical industry/universe coverage. Synthetic and public-development reports
remain `production_eligible: false`; paid historical-data admission stays deferred.

## Compatibility and output

| Path | Engine | Panel artifact | Output artifacts |
| --- | --- | --- | --- |
| Raw, empty transformation policies | `factor-evaluator.2` | `loop.factor_panel/v1` | values and manifest v1 |
| Explicit frozen transformations | `factor-evaluator.3` | `loop.factor_panel/v2` | values and manifest v2 |

The raw values CSV input stays version 1. A transformed dataset contains that
CSV, the version-2 panel manifest and an exposure CSV only when needed. The panel
appends `transform` with the two canonical policy documents and exact exposure
reference. Both Rust and Python bind them to the immutable FactorSpec. Rust also
requires every FactorSpec policy to match the frozen research context. Unknown
algorithms fail closed; raw evaluation rejects nonempty transformation settings.

The result retains the existing factor-value CSV columns. Its version-2 JSON
adds `transform` with `profile`, both policy digests, nullable exposure digest,
`raw_valid_observations` and session `outcomes`. Artifact schema identities and
source/environment provenance prevent a transformed result from being relabeled
as an earlier raw result. The Protobuf job and completion protocol are unchanged.

Rollback disables new version-2 writers/jobs and retains raw version-1 support.
Preserve all completed captures, construction receipts, factor identities,
results and audit history. No new service, dependency or database migration is
required. Exact replay uses the original frozen implementation/environment;
rebuilding with changed source code creates new provenance and receipts.
