# Local NAV correlation diagnostic

## Purpose

Review finding 8 concerned correlation computed from changes in NAV rather than
returns. The existing numerical kernel now has a read-only CLI consumer that
also rejects unequal observation dates. Two equal-length arrays do not establish
matching return intervals: both endpoints of every observation interval must
match. The command never tail-aligns, intersects, forward-fills or resamples.

This is a local diagnostic, not the primary backtester, a factor admission gate,
an authenticated result exporter, or an approved Agent tool. It has no database,
network, provider or holdout capability. Files remain subject to the invoking
user's filesystem permissions. Do not expose this command as a research tool
with unrestricted filesystem access.

## Run

From the repository root, run the deliberately synthetic examples:

```bash
.venv/bin/loop-research nav-correlation \
  fixtures/research/nav/left.csv \
  fixtures/research/nav/right.csv \
  --cash-flow-adjusted
```

Use Python 3.14.4 and the existing root uv workspace. No additional dependency
or virtual environment is required. `loop-research doctor` remains unchanged.

Each input must be a regular, non-symlink ASCII CSV file with exactly this
header and two fields per row:

```csv
session,nav
2020-01-02,100
2020-01-03,110
2020-01-06,99
```

Dates must be canonical `YYYY-MM-DD`, unique and strictly increasing. Both files
must have identical complete date sequences. The date labels are observations,
not a verified exchange calendar: shared gaps are allowed but the resulting
returns are interval returns, not necessarily daily returns. Calendar
completeness remains a data-plane integration gate.

NAV must be finite and nonnegative, with positive prior values. A terminal zero
is a total loss; a later observation after zero is rejected. Non-numeric values,
overflow and nonzero input values underflowing to float64 zero fail closed.
Both NAV paths must already adjust for external cash flows. The required
`--cash-flow-adjusted` flag is the caller's assertion, not accounting evidence.

Limits are 10 MiB and 100,000 observations per input; numeric cells have at most
64 ASCII characters. `--min-observations` counts return pairs, defaults to 5,
and must be between 2 and 99,999. No first-period zero return is invented.

## Output And Errors

One deterministic JSON report is printed only after both inputs validate. It
records SHA-256 of the exact bytes parsed, the observation endpoints/count,
return-pair count, requested minimum and Pearson correlation of
`NAV[t] / NAV[t-1] - 1`. It does not hash or identify a `BacktestResult`.

The report explicitly labels local data as `unverified_local_input`, calendar
validation as `not_performed`, and cash-flow adjustment as `caller_asserted`.
These fields must not be interpreted as a production provenance attestation.
Input-file digests do not replace the six-component execution fingerprint.

`status` is `ok`, `insufficient_observations` or `constant_returns`. Undefined
correlation is JSON `null`, never a fabricated zero or non-standard `NaN`.
An unavailable diagnostic is still a valid report with exit status 0. Invalid
inputs, inaccessible files or missing arguments produce exit status 2, an
error on stderr and no partial report on stdout. Reports are not automatically
stored, exported to a remote destination, or used for an admission decision.

## Verification And Rollback

Tests invoke the actual CLI in a fresh process, compare independent SciPy return
goldens, reject shifted endpoints and malformed inputs, preserve undefined
results, verify byte digests and prove the input files are not modified. FIFO
inputs cannot wait for a writer; byte and observation scans are bounded.

Roll back by reverting this additive CLI commit or deploying the previous
research package. Existing `doctor` behavior, numerical kernels, protocol IDs
and database schemas are unchanged. No migration or historical-data deletion is
needed; preserve any diagnostic reports already retained by callers.
