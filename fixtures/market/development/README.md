# Development adapter HTTP fixtures

These are invented responses shaped from the official SEC and Alpaca schemas.
The issuer, UUID, ticker, accession numbers, prices and fundamentals are synthetic.
They are not vendor downloads, valid entitlements or economic research evidence.
The unusual decimal values test exact JSON parsing; units and periods deliberately
exercise separate observation identities. No real credential is included.

Tests serve these bytes through an injected HTTP transport and write/replay real
cache files. Runtime requests still require the fixed HTTPS vendor routes; the
CLI has no fixture transport or arbitrary endpoint option.
