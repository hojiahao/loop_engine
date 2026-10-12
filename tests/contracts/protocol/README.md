# Protobuf wire compatibility fixtures

The Python, Rust, and TypeScript producers each construct the same
non-sensitive `loop.v1.ProtocolInfo` domain projection and commit their own
binary fixture. Consumers validate semantics; the contract deliberately does
not require different Protobuf implementations to emit identical bytes.

The additive fixture appends an unknown length-delimited field to model a
new-writer/old-reader exchange. Two `JobSpecification` negative fixtures model
an unknown enum value and a future oneof alternative that decodes as an empty
oneof in an older binding. Production DTO-to-domain validators must reject both
states with stable errors.

Generate or verify every fixture with all three pinned toolchains:

```bash
./scripts/wire-fixtures.sh --write
./scripts/wire-fixtures.sh --check
```

`scripts/proto-check.sh` runs the freshness check. The manifest binds each
fixture to its producer source, tool versions, descriptor, and SHA-256. Rust,
TypeScript, and Python decode every producer fixture, compare the known domain
projection, re-encode locally, and decode again. They do not require unknown
field retention. A component promising lossless forwarding retains the
original immutable envelope.

The fixture advertises only features backed by executable Phase 2 validators.
Model and stream messages are wire-shape definitions at this phase, so
`streams.terminal-event.v1` remains absent until the Phase 9 content validator
and Phase 10 consumer state machine pass their cross-language contract gates.

The `provider_lookup_v1.binpb` and `provider_completed_v1.binpb` fixtures are
produced by the pinned Python generator and consumed by Rust, TypeScript and
Python. They preserve the separate lookup/original request identities, exact
request digest and USD reservation while leaving actual charged cost absent.
The recovery capability identity is `provider.invocation-lookup.v1`; these wire
fixtures do not advertise it in older generic `ProtocolInfo` fixtures. Current
clients detect missing RPC support through `UNIMPLEMENTED`, not a negotiated
metadata feature flag, and must never fall back to paid generation.

The `discovery_execute_v1.binpb` and `discovery_completed_v1.binpb` fixtures
bind a fresh execute command to an expected revision and project one completed
step with a canonical AST candidate plus conservative token/USD reservations.
Rust, TypeScript and Python preserve those values without exposing a generic
job record, raw model output, lease or protected-data authority. The capability
identity `discovery.model-step.v1` is additive and is not advertised by the
older generic ProtocolInfo fixture. A completed candidate is not an admitted
factor or a claim that model charges equal the reserved ceiling.
