# Phase 2 verification: protocol baseline history

- Date: 2026-09-05 (Asia/Shanghai)
- Branch: `refactor/us-equities-loop-runtime`
- Status: accepted Phase 2 seed trust anchor; product release not claimed
- Current and baseline descriptor SHA-256:
  `27a38398e290caee3fb857063c0f2adbbe43a7d2d44322103dbf4b72535be979`

## Pre-release corrections

Phase 2 had not been committed, tagged, published, or consumed by a released
client when successive adversarial reviews found authority-boundary defects.
Preserving an unsafe precursor merely to claim compatibility would have made
that defect permanent. Each replacement was explicitly approved, and every
rejected descriptor is now retained as a content-addressed repository fixture.
The earlier claims that `fdbb...` and then `51a4...` were final were invalidated
by later review findings. The subsequent `4cd9...` candidate also contained an
over-broad data dependency. None is an accepted or released contract.

| Correction | Old SHA-256 | New SHA-256 | Reason | Historical fixture |
| --- | --- | --- | --- | --- |
| Discovery boundary | `041a396de9b59c4ceb3c40f160f7e2ddda404f362ea72ab6929e1314eed510d1` | `62b560d6df574b152aa5e6cdcbad46834da3ecb0a923ced6aaa47c0fd91246f3` | `StartDiscoveryResponse` returned generic `JobRecord`; its `JobSpecification` could reach internal `HoldoutBacktestJobInput`, and the TypeScript discovery entry point re-exported `job_pb` | `fixtures/contracts/protocol/history/041a396de9b59c4ceb3c40f160f7e2ddda404f362ea72ab6929e1314eed510d1.binpb` |
| Holdout plan boundary | `62b560d6df574b152aa5e6cdcbad46834da3ecb0a923ced6aaa47c0fd91246f3` | `fdbb927352b591c85a1ed054f3200a0234e98db643f79cd9ef9e6f30d7868563` | The consume RPC accepted a caller-selected `BacktestSpec` and budget, returned a full generic job, and did not bind approval to a complete frozen batch or a canonical period identity | `fixtures/contracts/protocol/history/62b560d6df574b152aa5e6cdcbad46834da3ecb0a923ced6aaa47c0fd91246f3.binpb` |
| Research response boundary | `fdbb927352b591c85a1ed054f3200a0234e98db643f79cd9ef9e6f30d7868563` | `51a4c938810cede021a2cc2682654a52fb811efc8a16fa7d9c2ced88d00529f1` | Research responses returned generic `JobRecord`, exposing its internal specification, holdout input, lease, and outcome graph; the TypeScript role entry point also used wildcard exports | `fixtures/contracts/protocol/history/fdbb927352b591c85a1ed054f3200a0234e98db643f79cd9ef9e6f30d7868563.binpb` |
| Research dependency closure | `51a4c938810cede021a2cc2682654a52fb811efc8a16fa7d9c2ced88d00529f1` | `4cd9d0e49e174f3a2920bbee19921f4f48b459aa48ac6a5927efa48722ff81f7` | Narrow Research messages still imported `backtest.proto`, whose generated module exposed `BacktestSpec` and locked `SampleWindow`; safe shared return/provenance types now live in dependency-leaf `research_common.proto` | `fixtures/contracts/protocol/history/51a4c938810cede021a2cc2682654a52fb811efc8a16fa7d9c2ced88d00529f1.binpb` |
| Development-data dependency closure | `4cd9d0e49e174f3a2920bbee19921f4f48b459aa48ac6a5927efa48722ff81f7` | `27a38398e290caee3fb857063c0f2adbbe43a7d2d44322103dbf4b72535be979` | Research and Discovery still imported broad `data.proto`; its generated dependency module exposed locked `SampleRole`, `SampleWindow`, and artifact-bearing `DataSnapshot`. `DevelopmentDatasetReference` now lives alone in dependency-leaf `development_data.proto` while retaining its full wire name | `fixtures/contracts/protocol/history/4cd9d0e49e174f3a2920bbee19921f4f48b459aa48ac6a5927efa48722ff81f7.binpb` |

The sequence made discovery and research own narrow development-reference
shapes, budgets, and job handles; made the freeze manifest own one
content-addressed batch evaluation plan; made the period ID canonical; removed caller specifications
from holdout consumption; and made the complete Research and Discovery
dependency closures incapable of importing generic job, backtest-specification,
broad data-plane, locked-window, or holdout contracts. The history guard pins
the complete expected archive inventory and verifies every fixture's filename
against its exact SHA-256 rather than relying on ephemeral `/tmp` paths. Opaque
snapshot IDs still require Phase 4/5 server-owned
role resolution and capability enforcement before persistence or execution;
this descriptor boundary does not claim to prove what an ID names.

## Baseline evidence

The candidate descriptor was generated from the checked-out `.proto` sources.
The following checks establish that the candidate baseline and generated
current descriptor are byte-identical and that Buf accepts the result against
itself. Because no earlier protocol contract has been released, this seeds the
future compatibility guard; it is not evidence of an old-release-to-current
migration:

```text
sha256sum fixtures/contracts/protocol/v1/schema.current.binpb
sha256sum fixtures/contracts/protocol/v1/schema.baseline.binpb
cmp fixtures/contracts/protocol/v1/schema.current.binpb \
    fixtures/contracts/protocol/v1/schema.baseline.binpb
./scripts/pnpm.sh exec buf breaking \
    --against fixtures/contracts/protocol/v1/schema.baseline.binpb
```

Both SHA-256 commands return the value at the top of this record.
`./scripts/proto-check.sh` additionally checks the pinned baseline digest,
verifies every historical descriptor by its content-addressed filename, rejects
a one-byte-tampered temporary baseline, checks deterministic regeneration,
descriptor dependency/message reachability, reserved injection fields, language
bindings, wire fixtures, and the typed operational-failure contract. It never
modifies the real baseline.

Correction-specific verification recorded before the full exit gate:

| Command | Result |
| --- | --- |
| `./scripts/proto-generate.sh --write` | passed; generated Rust, TypeScript, Python, and current descriptor updated |
| `cmp schema.current.binpb schema.baseline.binpb` | passed; candidate and explicit baseline are byte-identical |
| `buf breaking --against schema.baseline.binpb` | passed |
| `./scripts/verify-protocol-baseline.sh --self-test` | passed: pinned trust anchor, exact history inventory, tampered baseline/history rejection, and missing-history rejection |
| `tests/contracts/check_protocol_boundaries.py schema.current.binpb` | passed: the Research and Discovery closures use `development_data.proto` and exclude broad `data.proto` |
| Rust `discovery_boundary` and `research_boundary` targets | 7 passed |
| TypeScript discovery/research boundary targets | 6 passed |
| Python discovery/research boundary targets | 8 passed |

`./scripts/proto-check.sh` and the complete host gates passed on 2026-09-07.
Pushed implementation commit `0615d81` also passed all seven jobs in
[CI run 34101687394](https://github.com/hojiahao/loop_engine/actions/runs/34101687394),
including deterministic generation and a clean DaoCloud container sequence.
The digest above is therefore the accepted Phase 2 seed trust anchor, without
changing either descriptor during phase closure. The aggregate evidence is in
`docs/verification/phase-02-core-contracts.md`.

This is not a product release, a research freeze, or evidence of migration from
a released protocol. Breaking changes require a new package major and reviewed
migration; normal generation never advances the baseline.
