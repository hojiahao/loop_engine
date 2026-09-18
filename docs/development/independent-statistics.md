# Independent Alphalens statistical verification

This administrative workflow independently calculates frozen cross-sectional
statistics with Alphalens Reloaded, compares every field against a reconstructed
primary result, and publishes immutable differences and receipts. It supports
synthetic and public-development samples from 2007–2020. Passing this comparison
does not admit a factor, unlock a holdout, certify licensed data, or complete the
pending Zipline accounting and authorized reconciliation gates.

## Environment

Run `./scripts/bootstrap.sh` on a supported host. The main research environment
remains Python 3.14.4 with pandas 3.0.5. Alphalens 0.4.6 requires pandas below 3;
`python/alphalens_validation/uv.lock` independently pins pandas 2.3.3, NumPy 2.5.2
and SciPy 1.18.1. Its wrapper uses an ephemeral uv environment, never a second
project `.venv`, and cannot import the primary numerical package through its
declared dependencies. Initial installation needs package-registry access;
subsequent commands run offline:

```sh
./scripts/uv-alphalens.sh run --locked --offline loop-alphalens doctor
```

The doctor checks the tested runtime versions and reads the installed numerical
source/native-library bytes. It makes no market-data or LLM requests. Plotting
caches use a project-prefixed temporary directory that is removed on normal exit.

## Export, calculate and replay

First obtain an actual statistics receipt using the
[primary statistics workflow](portfolio-statistics.md). Keep the same evidence,
read-only factor view and private CAS directories. Replace the placeholders below
with real absolute paths and SHA-256 references:

```sh
./scripts/uv-research.sh run --locked --offline --no-sync loop-research alphalens-prepare \
  --statistics sha256:REPLACE_WITH_STATISTICS_RECEIPT \
  --evidence /absolute/private/evidence \
  --view /absolute/readonly/factor-view \
  --store /absolute/private/portfolio
```

The command actually reconstructs the frozen factor, portfolio and statistical
evidence, verifies their current source/environment and original artifact bytes,
then publishes the raw observation CSV and an input manifest. Its JSON output is
`{"sha256":"sha256:...","byte_size":...}`. Use that digest in the independent
process:

```sh
./scripts/uv-alphalens.sh run --locked --offline loop-alphalens run \
  --input sha256:REPLACE_WITH_INPUT_MANIFEST \
  --store /absolute/private/portfolio

./scripts/uv-alphalens.sh run --locked --offline loop-alphalens validate \
  --receipt sha256:REPLACE_WITH_INDEPENDENT_RECEIPT \
  --store /absolute/private/portfolio
```

`run` prints a receipt reference and its referenced artifacts. `validate` returns
the identical report after recalculation and byte comparison; it neither writes
files nor repairs corrupt objects. The store must be a canonical absolute path,
owned by the process user and mode 0700. References resolve only inside that CAS;
URLs and external paths in a manifest are not resolved.

| Artifact | Evidence |
| --- | --- |
| `build` | Actual validator/numerical dependency bytes and interpreter identity |
| `cross_sections` | Every signal date, label date, status, count, IC, Rank IC, groups, spread and monotonicity |
| `turnover` | Alphalens quantile membership turnover on the complete frozen session axis |
| `differences` | Every differing field, both values, reason and fixed tolerance |
| `summary` | Profile, coverage, disposition, interpretation and remaining gates |

| Exit | Meaning |
| --- | --- |
| 0 | Comparison accepted, or doctor succeeded |
| 2 | Invalid input, lineage, artifact, build or bounded execution failure |
| 3 | Immutable comparison report contains a mismatch |
| 4 | Immutable report has insufficient sessions or unavailable required statistics |
| 130 | Interrupted; no successful outcome should be inferred |

Check the exit code and `disposition`; a valid JSON report alone is not a pass.
Rejection here describes statistical disagreement, not a production factor vote.

## Exact comparison semantics

The input CSV contains raw frozen signal/eligibility and next-session open/close
prices. The validator independently computes open-to-close labels and frozen
direction, applies average-tie Spearman Rank IC through Alphalens, Pearson IC
through SciPy, and Alphalens arithmetic group means without demeaning. Equal-count
groups run worst to best; stable security IDs break membership ties. Uneven
groups receive the extra observations in the earliest groups. These group returns
are uncosted intraday diagnostics, not executable group portfolios.

The primary and independent paths share raw observations and export normalization.
They do not share primary return/rank/correlation kernels. This is not a second
vendor feed or an independent factor-expression implementation. Missing selected
prices invalidate the whole date; dates are not silently dropped. Constant
signals, constant labels and insufficient cross sections retain explicit statuses.
The final signal has no observed forward return and is excluded from the minimum
session count. All other required dates must have available statistics.

The versioned `alphalens-statistics.1` profile compares identities, counts and
statuses exactly. Finite metrics use absolute tolerance `1e-12` and relative
tolerance `1e-10`; missingness is compared explicitly. Operators cannot widen
tolerances on the command line. Quantile membership turnover is a separate
diagnostic: it is not compared with executed-notional portfolio turnover. The
full calendar preserves Friday-to-Monday, holiday and unavailable-date lags.

## Bounds, authority and rollback

The grid is bounded to 100,000 cells, 8,192 sessions and 4,096 securities. Input
metadata is bounded to 128 KiB, exported raw observations to 32 MiB, each referenced
object to 64 MiB and combined outputs to 64 MiB. Export and validation each have a
180-second cooperative deadline with clock-regression rejection; native calls and
filesystem operations are not hard real-time preemptible.

Outputs are published before the final receipt. Interruption may leave orphaned
content-addressed objects; it does not authorize a result. Existing bytes are never
overwritten. Source, dependency or input changes fail current replay; preserve
old artifacts as historical evidence. Build identity covers validator and numerical
code, not a complete hermetic OS attestation.

The standalone worker has no database identity and does not authenticate the
origin of a caller-created input manifest. Actual export verification is required;
Phase 8 unit 3 will bind receipts to registered runtime evidence. A private CAS
and offline invocation are not by themselves a process sandbox.

Disable the new administrative commands or revert this task commit to stop new
writes. Retain evidence, outputs and receipts. No database schema or production
admission policy changes are involved. Design: [ADR 0033](../adr/0033-independent-statistical-validation.md).
