# Synthetic point-in-time history

`history.json` is invented test data. No identifiers, prices or facts describe a
real security. `DEMO` is reused after the old issuer delists; the later issuer
has two separate share classes. One revenue fact is published after its fiscal
period and restated later. Its decimal value exceeds float64's exact integer
range to detect lossy conversion.

The repeated-digit source digests are explicitly synthetic declarations, not
checksums of vendor files or proof of licensed/PIT data quality. The diagnostic
computes real SHA-256 digests of the input bytes and normalized query result.
Provider raw-content verification belongs to the adapter/snapshot pipeline.
