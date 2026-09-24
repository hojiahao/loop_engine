# Initial threat model

## Protected assets

- LLM, market-data, cloud, and database credentials.
- Locked confirmation datasets and their access capabilities.
- Factor definitions, trial history, metrics, portfolio returns, and audit logs.
- Provider catalog integrity and executable plugin supply chain.
- Local workstation, network access, and paid API budgets.

## Trust boundaries

The Web/TUI/CLI clients, Rust control plane, TypeScript provider host, Python
research worker, plugin processes, external APIs, metadata database, and object
store are separate trust zones. A valid message is not automatically an
authorized operation.

## Primary threats and required controls

| Threat | Required control |
|---|---|
| Look-ahead or holdout leakage | Capability-denied storage path, separate manifests, immutable freeze record, access audit |
| Survivorship or point-in-time bias | Stable security IDs, inactive securities, delisting returns, `known_at` queries, quality gates |
| Model/tool prompt injection | Typed tools, least privilege, schema validation, no shell or holdout capability by default |
| Secret disclosure | Secret references, process-scoped injection, structured redaction, log/prompt scanning |
| Malicious or compromised plugin | Allowlist, lockfile hash, isolated process, minimal network/secret scopes, signed catalog metadata |
| Provider response confusion | Strict response schemas, bounded payloads, event-state validation, fail closed |
| Duplicate or lost work | Transactional transitions, idempotency keys, leases, heartbeats, revision checks |
| Cost runaway or infinite loop | Per-call/run/project budgets, wall-clock and step limits, cancellation, terminal reason |
| Audit tampering | Append-only events, hash chaining, restricted mutation, exported manifest signatures |
| Data poisoning or silent revisions | Immutable raw snapshots, checksums, anomaly tests, cross-source samples, automatic staleness |
| Unsafe custom endpoint | URL validation, explicit allowlist, TLS by default, loopback/private-network policy, redirect limits |

## Research-integrity controls

LLMs may propose typed factor ASTs and semantic reviews but cannot approve final
performance or access confirmation returns. Deterministic filters and accounting
run before independent verification. Infrastructure errors never become factor
rejections or approvals silently. All attempted canonical factor IDs count
toward multiple-testing controls, including failed and exceptional trials.

## Residual risks

No architecture can make incomplete market data institutional-grade, guarantee
future model behavior, or eliminate strategy overfitting. Live-provider and
licensed-data validation remain explicit release gates. Claims about investment
performance, vendor coverage, or affiliation are prohibited without evidence.
