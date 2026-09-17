# Portfolio statistics and multiple testing

`loop-research statistics-run` reconstructs actual factor and portfolio inputs,
computes statistics and publishes immutable evidence. `statistics-validate`
repeats the computation without changing files. These administrative tools
support synthetic/public-development IS or development samples; they do not
issue holdout capabilities, register runtime results or admit factors.

## Freeze the evaluation policy

Keep the existing nine policy roles and portfolio request. Before freezing the
FactorSpec/evaluation, add these sorted string settings to `evaluation_policy`:

```json
{
  "groups": "5",
  "hac_lags": "5",
  "minimum_coverage_bps": "9000",
  "minimum_cross_section": "10",
  "minimum_sessions": "60",
  "pbo_blocks": "8",
  "statistics_profile": "daily-statistics.1"
}
```

These are explicit example research assumptions, not recommended significance
thresholds for investment. Supported bounds: groups 2–10; minimum cross section
3–1000 and at least the group count; minimum sessions 8–8192; HAC lags 0–60 and
less than minimum sessions; CSCV blocks 4/6/8/10. Unknown settings are rejected.
Changing any setting requires a new FactorSpec and evaluation. The existing
coverage-only policy cannot generate statistics.

## Run and reconstruct

Obtain a portfolio receipt with the documented [backtest workflow](portfolio-backtest.md).
The statistics request contains exact references, not externally supplied scores:

```json
{
  "schema": "loop.statistics-request/v1",
  "backtest": {"sha256": "sha256:<portfolio receipt digest>", "byte_size": 123},
  "experiment": null
}
```

Replace placeholders with actual hashes/sizes. Keep the request in an existing
private runtime directory. Use the root workspace environment:

```sh
./scripts/uv-research.sh run --locked --offline --no-sync loop-research statistics-run \
  /absolute/private/statistics.json \
  --evidence /absolute/private/evidence \
  --view /absolute/readonly/factor-view \
  --store /absolute/private/portfolio

./scripts/uv-research.sh run --locked --offline --no-sync loop-research statistics-validate \
  --receipt sha256:REPLACE_WITH_STATISTICS_RECEIPT \
  --evidence /absolute/private/evidence \
  --view /absolute/readonly/factor-view \
  --store /absolute/private/portfolio
```

The existing private CAS (0700) and read-only leaf view (0555 with 0444 files)
rules apply. Portfolio receipts and statistical outputs live in `--store`;
work, input manifests, plans and experiment outcomes live in `--evidence`.
The commands make no network requests or database mutations. A source,
environment, input, ledger or policy change invalidates current replay; old
artifacts remain immutable historical evidence.

The returned receipt references five objects:

| Object | Contents |
| --- | --- |
| `summary` | Policy, quality, observation counts, return/drawdown/turnover, Sharpe and uncertainty |
| `cross_sections` | Every signal date, forward-label availability, IC/Rank IC, groups and spread |
| `portfolio` | NAV ratios, executed turnover, drawdown, gross/net and beta/log-size exposures |
| `exposures` | Signed industry market-value/NAV weights; absence is explained by daily status |
| `multiple_testing` | Family evidence, every trial, adjusted p-values, DSR and CSCV split diagnostics |

An unavailable statistic has `status="unavailable"`, `value=null`, an explicit
reason and an observation count. Constant returns do not yield infinite Sharpe;
missing labels do not become zero or disappear from the calendar. Without a
complete experiment, ordinary statistics are available where defined and
multiple testing explicitly reports `missing_experiment_evidence`.

## Register a finite trial family

This profile verifies a **declared finite family**, not a researcher's complete
prior/adaptive search. Registration here is an administrative immutable file;
its creation time and authorship are not authenticated. Phase 7 unit 4 must
connect the global durable trial registry before statistical admission.

1. Prepare each candidate's expression, direction, eight non-evaluation policies,
   sample, data, seed and execution tape. Use the existing raw-factor evaluation
   evidence to prepare its base portfolio request before running the planned
   portfolio batch. The final evaluation policy is added after plan publication;
   its new FactorSpec requires updated evaluation evidence in step 4.
2. Compute each binding with `statistics-bind`, using its portfolio request and
   serialized work. This is identity preparation, not evidence verification:

   ```sh
   ./scripts/uv-research.sh run --locked --offline --no-sync loop-research statistics-bind \
     /absolute/private/candidate-request.json --evidence /absolute/private/evidence
   ```

3. Publish a `loop.experiment-plan/v1` with `family_id` and an ordered `trials`
   array of `{trial_id, binding_sha256}`. There must be 2–64 unique IDs and unique
   bindings. Use the existing CAS `publish` function; no mutable checkpoint is added.
4. Add `experiment_plan` (the plan digest's 64 hex characters without `sha256:`)
   and `trial_id` to each candidate's evaluation-policy settings. All other
   statistical settings must match. Freeze and run the actual evaluation/backtest.
5. Publish `loop.experiment-evidence/v1` with `plan` (hash and byte size) and
   `outcomes` in exactly the planned order. Each outcome has `trial_id` and exactly
   one non-null `backtest` or `failure` reference. No trial may be missing,
   duplicated, reordered or replaced with another candidate's result.
6. A failure object has schema `loop.trial-failure/v1`, `trial_id`,
   `binding_sha256`, `kind` and `reason`. Kinds are `rejected`,
   `infrastructure_failure` and `cancelled`. Reasons are bounded codes, not logs
   containing private data. These are administrative declarations, not trusted
   runtime failure attestations.
7. Set the statistics request's `experiment` to the evidence reference. Run
   `statistics-run`. Every successful trial is reconstructed, including its
   frozen plan binding, policy, build, exact sample, data and output ledger.

Plan/outcome objects use canonical JSON from `build_identity.canonical_bytes` and
`data.fetch_cache.publish`. Python typed models in `statistics_models` validate
their exact format; unknown/duplicate JSON fields are rejected on read. Work and
execution bindings exclude job/lease IDs, artifact publication clocks, factor ID,
evaluation policy and the configuration fingerprint to avoid a circular hash.
They retain actual eight policy references, expression/direction, source/data/
calendar/environment, panel reference, sample, random seed and execution tape.
The final backtest reconstruction separately validates the complete FactorSpec
and all nine actual policy documents.

## Statistical interpretation

IC labels are raw **next-session open-to-close returns**, paired with the previous
close's finite eligible signals and frozen direction. This diagnostic deliberately
does not include the preceding overnight move, reinvested dividends or fees.
V2 supported corporate actions occur before the opening. A missing price for any
selected signal invalidates that whole cross section, preventing silent removal
of a halted/delisted name. The final signal has no forward session. Average ties
are used in Rank IC; equal-count groups use stable security IDs to break membership
ties. Groups run worst-to-best; group returns are arithmetic diagnostics, not
costed, executable portfolios. Constant signals produce no artificial groups.

Portfolio returns are `NAV[t]/NAV[t-1]-1`. Initial cash enters the drawdown peak.
One-way turnover is half absolute executed fill notional / previous NAV, including
opening trades. Exposures are signed position values / NAV; gross uses absolute
values. Beta, log USD market-cap and industries use the original causal panel;
missing held-security exposure data remains unavailable. Cash and action claims
are not allocated to equity industries.

Newey–West uses Bartlett weights, the frozen lag, n/(n−1) correction and asymptotic
normal two-sided p-values/95% intervals. It does not compress missing dates.
Daily Sharpe uses sample standard deviation and a disclosed zero daily benchmark.
The annualized display multiplies by sqrt(252); it is not serial-correlation
adjustment or an assertion that excess return over a real cash yield was measured.

BY-FDR uses every declared trial at alpha=0.05. Failed or undefined tests supply
p=1 and cannot pass; they remain visible and distinct from valid p-values.
Dependence robustness assumes valid marginal tests and a legitimate family.
DSR requires all complete trial returns; it uses the full trial count as an
explicit independence assumption, sample variance across daily Sharpes, and
biased central skew/Pearson kurtosis. This does not estimate effective independent
trials or correct serial correlation. The result is an assumption-dependent
diagnostic, not a probability of investment success.

CSCV uses all half-block combinations of equal contiguous blocks and synchronous
costed portfolio returns. It selects highest training Sharpe, with first plan
order for exact ties, and uses average test ranks divided by `trials+1`. PBO is
the fraction of logits <=0. Missing trials, degenerate split Sharpes, short samples
or nondivisible block lengths return unavailable; no data or splits are dropped.
CSCV partitions previously simulated returns; it does not retrain strategies,
provide independent OOS evidence or unlock protected samples.

## Bounds and rollback

Metadata objects are at most 128 KiB, trial count at most 64, aggregate replay
grid/return matrix at most 100,000 cells, CSCV blocks at most 10 (252 splits),
and aggregate output at most 64 MiB. The existing 180-second cooperative deadline
also covers family reconstruction and split checks; native calls are not hard
real-time preemptible. No existing protection is relaxed to make a test pass.

Disable statistics writers or revert the task commit to roll back. Preserve
plans, raw data, ledgers, statistics, receipts and audit history. There is no
database migration, destructive down-migration or implicit cache repair.
Method sources and engineering decisions are recorded in [ADR 0031](../adr/0031-statistical-evaluation.md).
