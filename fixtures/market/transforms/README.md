# Invented cross-sectional transformation input

All identifiers, prices, exposures and source evidence are synthetic. These
files contain no downloaded market data, credentials or performance claims.

- `source.json` is the original invented evidence.
- `capture.json` contains eight security histories and 24 daily raw bars over
  the 2010-01-04 through 2010-01-06 XNYS sessions.
- `exposures.json` contains timestamped industry, USD capitalization and beta
  observations backed by that source.
- `transform.json` freezes unstandardized equal-weight OLS on all three
  exposures, no winsorization and at least four observations.
- `request.json` binds all inputs for the version-2 panel builder.

Across the eight securities, log size is `[-1,1,-1,1,-1,1,-1,1]`, beta is
`[-1,-1,1,1,-1,-1,1,1]`, and the first/last four belong to industries A/B.
The residual is `[1,-1,-1,1,1,-1,-1,1]`, orthogonal to the intercept and each
exposure. Raw closes are `100 + day*2 + log_size*2 + beta*4 + industry_B*7
+ residual`, with day indexed from zero. Capitalization is the explicit decimal
representation of exp(log size). The independent numerical tests allow only
binary64 rounding error in these hand-derived residuals.

The installed Python worker returns 24 valid transformed field values. The Rust
mTLS/PostgreSQL integration evaluates a two-observation moving average: its
first session is missing and the remaining 16 cells match the same residual.
The eligible denominator remains 24. Restart replays the immutable completion;
altered exposure bytes or inconsistent frozen policies prevent execution.

Tests publish source files by their exact digest before invoking `panel-build`.
Any fixture edit must update dependent digest/size references. Run the transform
pipeline tests and the manifest/runtime suite to verify both language boundaries.
