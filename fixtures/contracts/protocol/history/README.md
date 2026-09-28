# Unreleased protocol descriptor history

This directory preserves rejected, pre-release Phase 2 descriptor candidates.
No file here is an accepted compatibility baseline or evidence of a released
wire contract. Each `.binpb` filename is the SHA-256 of its exact bytes, and
`scripts/verify-protocol-baseline.sh` verifies that binding on every protocol
check.

| SHA-256 | Why the candidate was rejected |
| --- | --- |
| `041a396de9b59c4ceb3c40f160f7e2ddda404f362ea72ab6929e1314eed510d1` | Discovery returned the generic durable job record, making internal holdout input reachable. |
| `62b560d6df574b152aa5e6cdcbad46834da3ecb0a923ced6aaa47c0fd91246f3` | Holdout consumption still accepted caller-selected specifications and budgets. |
| `fdbb927352b591c85a1ed054f3200a0234e98db643f79cd9ef9e6f30d7868563` | Research responses still returned the generic durable job record. |
| `51a4c938810cede021a2cc2682654a52fb811efc8a16fa7d9c2ced88d00529f1` | Research owned narrow inputs and responses, but its generated dependency surface still imported `backtest.proto`, exposing locked sample-window types. |
| `4cd9d0e49e174f3a2920bbee19921f4f48b459aa48ac6a5927efa48722ff81f7` | Research no longer imported `backtest.proto`, but both Research and Discovery still imported the broad `data.proto`, whose generated dependency module exposed locked sample roles, `SampleWindow`, and artifact-bearing `DataSnapshot`. |

The current trust-anchor candidate remains at
`fixtures/contracts/protocol/v1/schema.baseline.binpb`. Advancing that trust
anchor is a separately reviewed release operation; ordinary generation cannot
overwrite it.
