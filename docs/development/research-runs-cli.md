# Persistent research run CLI

`loopctl run` operates one human-owned, administrator-pinned research run. The
default version-0 workflow shares a durable budget across its Discovery children. It does
not perform numerical evaluation, semantic checking, factor admission or
unattended scheduling; those are separate integrations.

## Operator configuration

The server administrator registers two independent identities: a Human operator
who owns the run and a Discovery Agent whose frozen plan executes its children.
An actor label in JSON is attribution, not authentication: the server also checks
the presented mTLS certificate against its identity registry. The operator gets
no database password, Provider credentials or holdout capability.

Use the explicit `loop.operator/v1` profile. The existing `loop.client/v1`
Discovery profile is Agent-only and cannot be used with these commands. A Human
profile is likewise rejected by `loopctl discovery`.

```json
{
  "schema": "loop.operator/v1",
  "endpoint": "https://127.0.0.1:18443",
  "server_name": "loopd.internal",
  "ca_file": "/srv/loop/operator/ca.pem",
  "certificate_file": "/srv/loop/operator/human.pem",
  "private_key_file": "/srv/loop/operator/human.key",
  "actor": {
    "actor_id": "actor.operator.hojiahao",
    "subject": "human:hojiahao",
    "display_name": "hojiahao"
  }
}
```

These paths and identity names are examples, not claims that the operator has
been provisioned in production. Configuration and private key files must have
mode `0600` and be owned by the current user or root. All files use absolute,
canonical paths without symlinks; certificates cannot be writable by group or
others. Unknown JSON fields, insecure endpoints and TLS verification bypasses
are rejected. Transport/server error details are never printed.

## Administrator plan preparation

First prepare a frozen Discovery plan using the
[Discovery plan guide](durable-model-step.md), or its
[controlled-context variant](controlled-context.md). The new run uses that
plan's exact run ID, filled `DiscoveryJobInput`, protocol selection and outbound
Agent identity. The source Discovery input template omits `research_policy`;
the input copied into a run specification must include the reference derived
from the published Discovery plan. Do not confuse this reference with the
separate run-plan reference below. Prepare a new run before submitting any
standalone Discovery jobs with its ID.

Register the Human owner's distinct actor and client-certificate fingerprint in
the deployment identity registry with `role: "operator"` and the explicit run
ID. The Discovery identity keeps `role: "discovery"`, the same run ID and the
actual configured Provider connector actor. These identities must have different
actor IDs and authenticated subjects; an existing Agent certificate cannot
become a Human identity by editing the CLI JSON. Identity validity intervals
must cover the intended execution period.

Create a separate private flat directory, for example
`/srv/loop/private/run-plans`, with mode `0700`. Prepare a canonical binary
`loop.runs.v1.RunSpecification` using the generated bindings:

| Field | Required value |
| --- | --- |
| `plan` | Omit from the stored template to avoid circular hashing |
| `run_id` | Exact run ID in the existing Discovery plan |
| `owner` | Full registered Human actor, including kind, ID, subject and display name |
| `executor` | Full registered Agent actor used by the Discovery Provider connector |
| `discovery` | Exact filled `DiscoveryJobInput`, including its research policy |
| `protocol_selection` | Exact selection stored in the Discovery plan |
| `maximum_rounds` | Integer from 1 to 64 |
| `budget` | Explicit cumulative steps, input/output tokens, exact USD and wall-time ceilings |

Run wall time is positive, at most 30 days, and uses millisecond precision. Each
child keeps its original budget. For a two-round run, total token/step/USD
ceilings must fit two full child allowances; allow sufficient wall time for both
children plus queue and orchestration time. A budget that fits only one child
stops without creating a second one. This is a conservative reservation policy,
not a prediction of actual token usage or fees.

Serialize the template with `SerializeToString(deterministic=True)` in Python,
or the equivalent generated Protobuf encoder. The import is
`from loop.runs.v1 import service_pb2`; shared IDs, actors, money and policy
references come from `loop.v1.common_pb2`. Compute ordinary SHA-256 of the exact
template bytes. Publish them once at `<plan_store>/<64-character hex digest>`
with mode `0600` or `0400`, and record their exact byte count. Refuse an existing
file with different bytes; never overwrite an immutable plan object.

Next publish this closed JSON wrapper in the same content-addressed directory:

```json
{
  "schema": "loop.research-run/v1",
  "id": "research.baseline",
  "revision": "1",
  "specification": {
    "sha256": "sha256:<template digest>",
    "byte_size": 1234
  }
}
```

Replace the digest and byte count with the template's actual values. The ID uses
lowercase policy-ID syntax; revision is a positive canonical decimal integer.
Hash the exact UTF-8 wrapper bytes, then publish those bytes at the wrapper's
own hex-digest filename. Formatting affects this byte hash: do not reformat the
wrapper after publication. Each object is at most 1 MiB.

Add the wrapper reference to the existing private `loop.runtime/v1` JSON:

```json
{
  "runs": {
    "plan_store": "/srv/loop/private/run-plans",
    "plans": [{"sha256": "sha256:<wrapper digest>", "byte_size": 456}]
  }
}
```

This is an additional section, not a complete runtime configuration. Preserve
its existing identity, TLS, storage and `discovery` sections. The catalog accepts
1–64 plans and rejects duplicate run IDs. A run catalog requires the matching
Discovery executor. Restart the service after adding the plan; loading validates
content, identity and exact Discovery bindings without making a model call.

Finally prepare the operator's private `run-plan.binpb`: encode a
`loop.v1.PolicyReference` with the wrapper's ID, revision and raw 32-byte wrapper
SHA-256. This is the file consumed by `loopctl run start --plan`; it is **not**
the JSON wrapper or the `RunSpecification` template. The server fills that same
reference into its resolved specification. Keep all these configuration files
outside Git and outside research data/output directories.

## Start, step and observe

The administrator provides a private canonical binary `loop.v1.PolicyReference`
file identifying a registered immutable run plan. It contains `policy_id`, a
positive decimal revision and the 32-byte SHA-256 digest. Its encoding must match
the generated Protobuf encoder exactly; unknown or duplicate fields and
nonminimal encodings fail before network access. The file must be absolute,
regular, at most 4 KiB, non-symlinked and mode `0600`. It is a reference, not a
replacement plan or a way to override the frozen budget.
The registered plan also fixes the run ID. Start refuses to adopt pre-existing
unbudgeted jobs with that ID; supplying a different command key cannot replace
the plan or replenish an existing run's allowance.

```bash
loopctl run --config /srv/loop/operator/run-client.json start \
  --plan /srv/loop/operator/run-plan.binpb --key start.research.001

loopctl run --config /srv/loop/operator/run-client.json status \
  --run run.research.001

loopctl run --config /srv/loop/operator/run-client.json step \
  --run run.research.001 --revision 1 --key step.research.001
```

Use the run ID and revision returned by the preceding response. The example
revision is not a promise that later runs use the same value. Start reserves the
first entire child allowance. Each step executes or recovers the current child
and advances at most one round. A successful child may reserve and queue the
next child; a separate explicit step is needed to execute that child. There is
no automatic loop or retry in the CLI.

For a lost response, retry the exact original command, key and revision.
Receipts return the original command's historical result, even if the run has
since advanced; use `status` for its current state. They prevent that retry from
authorizing another round. Never generate a new
key or update the revision automatically to circumvent a conflict. Status reads
are observational and do not reset budgets. Plan changes or inaccessible
execution evidence fail closed; authorized stop-only status can return
`plan_verified: false` without exposing candidates or granting execution.

## Output and budgets

Successful transport responses print one `loop.run-cli/v1` JSON document to
stdout. The `run` object includes status, revision, frozen maximum rounds,
completed rounds, current child handle, original budget, cumulative reservations,
submission/update/deadline timestamps and `plan_verified`.

All integer fields are decimal strings, including revisions, token counts,
round counts and timestamp components. USD values remain exact canonical decimal
strings. The CLI rejects unknown status values, inconsistent projections and
reservations exceeding their ceilings instead of printing a misleading result.

Reservations are conservative ceilings, not billing totals. The complete frozen
child budget is reserved once at the run level; individual model steps consume
that child's budget. Unused or ambiguous calls do not refund parent reservations.
Both restart and repeated commands preserve the original absolute deadline.

| Exit code | Meaning |
| --- | --- |
| 0 | Valid active or completed run response |
| 2 | Invalid arguments, configuration or plan input |
| 3 | Authentication or authorization denied |
| 4 | Revision, idempotency or state conflict |
| 5 | Client/RPC deadline reached |
| 6 | Transport/server evidence unavailable, rate limited or remotely cancelled |
| 7 | Run exhausted its budget, failed infrastructure, or exceeded its deadline |
| 8 | Missing, corrupt or unsupported response, or internal client failure |
| 130 | Client interrupted by SIGINT |

Errors print a redacted `loop.run-cli/v1` envelope to stderr. A terminal run
response still prints its safe projection to stdout even when it exits with 7.
SIGINT only drops the client RPC; it does not claim to cancel the durable run or
refund reservations. For the default workflow, `--timeout-seconds` bounds connection plus RPC to 1–120
seconds (default 30); a deadline does not authorize resending a model invocation.

## Numerical workflow (opt-in)

Version 2 selects the candidate-to-evaluation and portfolio workflow. The
installed service, database, worker, restart and remote acceptance for delivery
11.2 are recorded in the
[numerical workflow evidence](../verification/phase-11-numerical-workflow.md).
It does not enable formal factor admission.

The default `--workflow-version 0` preserves existing discovery-only plans,
request bytes and `loop.run-cli/v1` output. Select `--workflow-version 2` for
every start, step and status request against a numerical plan; the version must
match the stored plan. A version flag cannot convert an existing run or grant
Discovery access to Research or holdout data. Version 3 additionally requires
the frozen feedback policy described below; versions 0 and 2 remain unchanged.
The numerical executor is a separately registered Research **Service** identity,
distinct from both the Human owner and Discovery Agent. Its frozen protocol uses
`loop.v1` and exactly `jobs.envelope.v1`, `jobs.kind-input.v1` and
`jobs.prelease-terminal.v1`; copying Discovery or Provider capabilities into
this selection is rejected.

### Prepare the numerical inputs

Prepare a fresh, unused run ID and its Discovery plan first. Both the Discovery
input and numerical recipe must identify the same complete dataset manifest and
ordered snapshot IDs. An existing run cannot be converted to v2. Use the
[causal panel workflow](causal-factor-panels.md),
[factor evaluation guide](factor-evaluation.md) and
[authorized portfolio guide](authorized-portfolios.md) to publish:

- The six-component `loop.research-context/v1` manifest and every referenced
  source, registry, configuration, data, calendar and environment object.
- One explicit factor-panel snapshot, its actual value objects and any declared
  transformation exposures, plus the execution snapshots used by the portfolio.
- A `loop.execution_tape` version-1 artifact with `application/json` media type,
  registered inside that same frozen dataset. A loose tape file is insufficient.
- Nine actual policy documents, including the frozen coverage, portfolio,
  execution, cost and statistical settings required by the installed workers.

The context must use the `in_sample` role within the frozen 2007–2016 boundary,
the primary cross-sectional engine, and `factor-evaluator.2` for a raw panel or
`factor-evaluator.3` for a transformed panel. It must not already contain a
`portfolio_request`. Development validation, warmup-only and protected samples
are rejected by this workflow, even though other authorized numerical workflows
may support development validation. Snapshot `known_through_ms` cannot exceed
the sample end. Synthetic and public-development fixtures do not establish
licensed, survivorship-aware production data quality.

Publish this recipe in the private run-plan object store. Replace every digest
and byte count with the actual referenced object; the displayed sizes are
examples. Policy values must be exact IDs from the context configuration,
distinct across all nine roles; their names need not use the `policy.` prefix.

```json
{
  "schema": "loop.run-numerical/v1",
  "id": "recipe.us.research",
  "revision": "1",
  "evaluation_context": {"sha256": "sha256:<context digest>", "byte_size": 900},
  "direction": "higher_is_better",
  "deterministic_seed": "sha256:<fixed 64 lowercase hexadecimal digits>",
  "execution_tape": {"sha256": "sha256:<registered tape digest>", "byte_size": 320},
  "policies": {
    "calendar_policy": "policy.calendar",
    "cost_policy": "policy.cost",
    "data_policy": "policy.data",
    "evaluation_policy": "policy.evaluation",
    "execution_policy": "policy.execution",
    "neutralization_policy": "policy.neutralization",
    "portfolio_policy": "policy.portfolio",
    "preprocess_policy": "policy.preprocess",
    "universe_policy": "policy.universe"
  },
  "hypothesis": "The preregistered signal ranks the declared IS cross section.",
  "required_observations": ["Causal daily prices and next-session execution observations."],
  "falsification_criteria": ["Coverage or economic evidence fails the frozen research policy."]
}
```

`direction` accepts only `higher_is_better` or `lower_is_better`. There is no
automatic direction selection. The exact 32-byte seed remains fixed across
rounds so renaming jobs or advancing the run cannot evade duplicate detection.
The hypothesis is nonempty; each observation/criterion list has 1–16 entries.
Every narrative entry is at most 2,048 UTF-8 bytes, has no control characters or
leading/trailing whitespace. These are administrator declarations, recorded as
recipe-declared claims, not model reasoning or evidence that a factor works.

The recipe parser rejects unknown fields and requires its compact canonical
encoding. Preserve the top-level field order shown above, the `ObjectRef` order
`sha256`, `byte_size`, and lexicographically sorted policy-role keys; encode as
UTF-8 without insignificant spaces or a trailing newline. For example, after
replacing placeholders in a private `numerical-recipe.source.json`, use the
installed project interpreter to construct the bytes:

```python
import hashlib
import json
from pathlib import Path

source = json.loads(Path("/srv/loop/private/numerical-recipe.source.json").read_text())
fields = (
    "schema", "id", "revision", "evaluation_context", "direction",
    "deterministic_seed", "execution_tape", "policies", "hypothesis",
    "required_observations", "falsification_criteria",
)
if set(source) != set(fields):
    raise ValueError("unexpected or missing recipe fields")
recipe = {name: source[name] for name in fields}
for name in ("evaluation_context", "execution_tape"):
    ref = recipe[name]
    if set(ref) != {"sha256", "byte_size"}:
        raise ValueError("invalid object reference")
    recipe[name] = {"sha256": ref["sha256"], "byte_size": ref["byte_size"]}
recipe["policies"] = dict(sorted(recipe["policies"].items()))
recipe_bytes = json.dumps(recipe, ensure_ascii=False, separators=(",", ":")).encode()
recipe_ref = {
    "sha256": "sha256:" + hashlib.sha256(recipe_bytes).hexdigest(),
    "byte_size": len(recipe_bytes),
}
```

Publish `recipe_bytes` once using the no-overwrite object-publication procedure
above and retain `recipe_ref`. The context and tape remain in the development
artifact store; only the small recipe belongs in the run-plan store. An object
reference is always a digest **and** an exact byte count, never a caller path.

### Register the Research service

Add a separate entry to the runtime `identities` array. This is an array element,
not a new top-level `research_identity` configuration property:

```json
{
  "actor_id": "service.research.us",
  "subject": "service:research:us",
  "display_name": "US numerical research",
  "role": "research",
  "certificate_sha256": ["sha256:<Research leaf certificate DER digest>"],
  "not_before_ms": 1791763200000,
  "expires_at_ms": 1794441600000,
  "run_ids": ["run.numerical.001"]
}
```

Replace the example dates with the approved execution interval and the digest
with the actual independently provisioned certificate. The server checks
identity validity at execution time; changing the display name, subject or
actor in the frozen plan requires a new plan. Numerical children use an
internally scoped Research service delegation. The Human operator is never
turned into a Research principal, and the Discovery Agent never receives this
service's certificate, numerical access or holdout capability.

For a deployment that operates only dynamically reserved run children, the
existing `jobs` and `data` arrays may be empty. Add both workers and retain the
existing TLS, identity, Discovery and storage sections:

```json
{
  "jobs": [],
  "data": [],
  "evaluation": {
    "python": "/srv/loop/runtime/python/bin/python3.14",
    "output_store": "/srv/loop/private/evaluation-output",
    "contexts": []
  },
  "portfolio": {
    "python": "/srv/loop/runtime/python/bin/python3.14",
    "output_store": "/srv/loop/private/portfolio-output",
    "jobs": []
  }
}
```

Replace the example interpreter with the installed, canonical absolute path;
do not register a symlink or create another environment merely to match this
example. Empty static pins do not enable generic
jobs: only a verified current run transition can supply a dynamic pin. Preserve
existing static pins if this deployment also serves standalone numerical jobs.
The development, protected, view and two output roots must be separate;
run plans must overlap none of them. Output and development directories are
owned by the runtime UID with mode `0700`. The development object store also
receives controlled, immutable derived manifests and verified copies of factor
outputs; no existing market-data object is overwritten. The service account
must therefore be able to publish new objects there. Both installed numerical
workers are required before a v2 run can reserve its first child.

### Encode the v2 run template

Use the same base fields as the v1 template, with `plan` omitted. Set
`RunSpecification.numerical` with these exact fields:

| Field | Binding |
| --- | --- |
| `research_executor` | Full registered Research actor with `kind = ACTOR_KIND_SERVICE` |
| `recipe` | `PolicyReference` for the recipe's ID, revision and raw SHA-256 |
| `evaluation_budget` | Positive steps/wall time; explicit token and exact USD ceilings |
| `portfolio_budget` | Positive steps/wall time; explicit token and exact USD ceilings |
| `protocol_selection` | Separate installed numerical protocol snapshot described below |

Both budgets use `loop.research.v1.ResearchJobBudget`. Numerical work makes no
model calls, so set input/output token ceilings and USD cost to zero. Retain a
positive step ceiling and enough wall time for verification, actual worker
execution and completion. The evaluator is bounded to 60 seconds of worker time;
the portfolio worker to 180 seconds. Those are limits, not promised runtimes.
If nonzero token ceilings are configured, they remain part of the frozen child
budget and parent reservation; the runtime does not silently replace them.
The parent must fit complete Discovery, evaluation and portfolio reservations
for each intended round, plus elapsed orchestration time. No child receives a
clipped budget and no unused reservation is refunded. A filtered candidate can
finish a round without creating a numerical child.

Prepare the numerical protocol from actual deployed client/server build pins
and the compatible descriptor, using the same fields and bounds documented in
the Discovery guide. Set `selected_package` to `loop.v1`, set exactly the three
sorted numerical features below, then recompute the domain-separated selection
digest. Do not hash Protobuf bytes as the selection identity:

```python
from loop.v1 import common_pb2
from loop_protocol.job import protocol_selection_sha256

numerical_protocol = common_pb2.ProtocolSelectionSnapshot()
numerical_protocol.CopyFrom(approved_protocol)
numerical_protocol.selected_package = "loop.v1"
del numerical_protocol.enabled_features[:]
numerical_protocol.enabled_features.extend([
    "jobs.envelope.v1", "jobs.kind-input.v1", "jobs.prelease-terminal.v1",
])
numerical_protocol.selection_sha256.value = protocol_selection_sha256(numerical_protocol)
```

Here `approved_protocol` is the generated snapshot prepared for the actual
deployment, with a non-future selection time, exact descriptor/build hashes and
valid effective limits. It is not a discovery permission grant. Use
`from loop.runs.v1 import service_pb2 as runs` and
`from loop.research.v1 import service_pb2 as research` for the generated workflow
and budget messages. For example, extend a fresh base template prepared using
the first section of this guide. The private `run-base.binpb` already contains
the exact Discovery input, actors, protocol, run ID and approved parent budget;
it has never been registered or started as a v1 run. The example child budgets
are administrative ceilings, not measured performance estimates:

```python
from google.protobuf.duration_pb2 import Duration
from loop.runs.v1 import service_pb2 as runs
from loop.research.v1 import service_pb2 as research

specification = runs.RunSpecification()
specification.ParseFromString(Path("/srv/loop/private/run-base.binpb").read_bytes())
if specification.HasField("plan") or specification.HasField("numerical"):
    raise ValueError("base template must omit plan and numerical")
specification.numerical.CopyFrom(runs.NumericalWorkflow(
    research_executor=common_pb2.Actor(
        actor_id=common_pb2.ActorId(value="service.research.us"),
        kind=common_pb2.ACTOR_KIND_SERVICE,
        display_name="US numerical research",
        authenticated_subject="service:research:us",
    ),
    recipe=common_pb2.PolicyReference(
        policy_id=common_pb2.PolicyId(value=recipe["id"]),
        revision=recipe["revision"],
        sha256=common_pb2.Sha256Digest(value=hashlib.sha256(recipe_bytes).digest()),
    ),
    evaluation_budget=research.ResearchJobBudget(
        maximum_steps=1, maximum_input_tokens=0, maximum_output_tokens=0,
        maximum_cost=common_pb2.Money(
            currency_code="USD", amount=common_pb2.ExactDecimal(value="0"),
        ),
        maximum_wall_time=Duration(seconds=120),
    ),
    portfolio_budget=research.ResearchJobBudget(
        maximum_steps=1, maximum_input_tokens=0, maximum_output_tokens=0,
        maximum_cost=common_pb2.Money(
            currency_code="USD", amount=common_pb2.ExactDecimal(value="0"),
        ),
        maximum_wall_time=Duration(seconds=240),
    ),
    protocol_selection=numerical_protocol,
))
template_bytes = specification.SerializeToString(deterministic=True)
template_ref = {
    "sha256": "sha256:" + hashlib.sha256(template_bytes).hexdigest(),
    "byte_size": len(template_bytes),
}
```

Publish `template_bytes` once in the same private run-plan store. Then publish
this v2 wrapper, using the computed `template_ref` and `recipe_ref` values:

```json
{
  "schema": "loop.research-run/v2",
  "id": "research.us.numerical",
  "revision": "1",
  "specification": {"sha256": "sha256:<v2 template digest>", "byte_size": 2345},
  "numerical_recipe": {"sha256": "sha256:<recipe digest>", "byte_size": 1200}
}
```

The recipe digest in this wrapper must equal the template's
`numerical.recipe.sha256`. A v1 wrapper must omit `numerical_recipe` and its
template must omit `numerical`; both are mandatory for v2. Add the v2 wrapper's
full reference to `runs.plans`, restart the compatible service, and encode the
operator's private `PolicyReference` from this wrapper's ID/revision/hash. That
small binary reference is the `--plan` file in the following commands.

### Execute and inspect

```bash
loopctl run --config /srv/loop/operator/run-client.json --workflow-version 2 start \
  --timeout-seconds 120 --plan /srv/loop/operator/numerical-plan.binpb --key start.numerical.001

loopctl run --config /srv/loop/operator/run-client.json --workflow-version 2 step \
  --timeout-seconds 240 --run run.numerical.001 --revision 1 --key step.numerical.001

loopctl run --config /srv/loop/operator/run-client.json --workflow-version 2 status \
  --run run.numerical.001
```

One explicit step executes or recovers at most one child and commits at most
one stage transition. Numerical steps accept a total client deadline up to
240 seconds, to cover the bounded 180-second portfolio worker plus orchestration.
Other commands and the legacy workflow retain the 120-second ceiling. The
default remains 30 seconds; no command retries automatically or replenishes
budget. After an uncertain response, reuse the exact version, key and revision.

The original parent deadline also bounds input preparation, numerical execution
and completed-child verification. If it expires during that work, the current
call returns an unavailable error; a subsequent explicit `step` records the
durable `deadline_exceeded` run status. The CLI does not issue that next command
automatically. A retry that finds an already committed command receipt returns
the original historical projection without launching a worker. This receipt
replay differs from recovering a completed child whose parent transition has
not committed, which must still verify its actual evidence.

Run state, transition ancestry and the global trial ledger are read within
PostgreSQL `REPEATABLE READ READ ONLY` transactions. A concurrent commit cannot
mix old parent state with newer children or trial history in one response.
Unavailable or unauthorized historical evidence denies the complete ledger;
the reader does not silently remove those trials.

Numerical responses use `loop.run-cli/v2`. Its `run.research` object contains
`round`, `stage` (`discovery`, `evaluation`, `portfolio`), a typed current-child
projection, source job IDs, factor identities, the immutable binding digest and
the round disposition. The legacy `run.current_job` field is absent in v2.
All integers remain decimal strings. The view never contains input data, an AST,
lease or capability. `evaluated` means portfolio evidence was registered; it is
not factor admission, a positive finding or independent backtest agreement.

## Adaptive workflow (opt-in)

Version 3 adds verified IS feedback and leased numerical window perturbation to
the same evaluation/portfolio path. Delivery 11.3a is **in progress**: the code
and examples describe the implemented interface, while installed acceptance,
complete workspace/container CI, publication and release remain unverified.
Consult the [adaptive acceptance record](../verification/phase-11-adaptive-workflow.md)
for actual evidence. These commands do not enable formal admission, autonomous
scheduling or live trading.

### Freeze the feedback policy

Prepare a new run using the v2 prerequisites above: a new run ID, independent
Human/Discovery/Research identities, installed numerical workers, an immutable
IS context and the complete frozen numerical recipe. Do not upgrade an existing
v0/v2 run or change its historical requests. Publish a compact canonical policy
in `runs.plan_store`:

```json
{
  "schema": "loop.run-feedback/v1",
  "id": "feedback.us.baseline",
  "revision": "1",
  "proposal_schedule": ["discovery", "perturbation", "discovery"],
  "window_path": [1],
  "eligible_windows": [2, 3, 4, 5],
  "random_seed": "sha256:0101010101010101010101010101010101010101010101010101010101010101",
  "max_observations": 16,
  "max_bytes": 16384
}
```

The displayed seed is an explicit synthetic example; choose and freeze the
research seed before search. Preserve the top-level field order shown, encode
UTF-8 with no insignificant whitespace or trailing newline, and hash/publish
the exact bytes using the no-overwrite procedure above. Unknown fields fail
validation. Keep the resulting `policy_bytes`, `policy_ref` and policy ID/revision
when constructing the template.

The schedule has exactly `maximum_rounds` entries, between 1 and 64, and begins
with `discovery`. Each entry replaces only the proposal stage: a round still
contains at most proposal, evaluation and portfolio. `[1]` selects the second
argument of the root AST call, for example the window of `ma(market.close,5,2)`.
Nested argument indices are supported; only an allowed window position in a
verified canonical expression can establish a perturbation family. The list of
eligible windows must be strictly increasing, contain 2–64 integers, and keep
each window within 1–4096. A malformed or semantically invalid candidate cannot
be made valid by the schedule.

The first eligible model candidate fixes the run-local family: the exact AST
except for that window, frozen direction, data, policies, seeds and numerical
build. Later model candidates may belong to another family and still contribute
verified feedback. If a scheduled perturbation has no established family with
an eligible finite Sharpe observation, the server freezes a Discovery fallback
before reserving the next child; it consumes no random draw. Size the parent
token/USD ceilings for that possible model call, or accept a budget stop.

### Encode and register v3

Extend the fresh, unregistered numerical specification; retain every v2 binding.
Set `RunSpecification.feedback_policy` to the policy's `PolicyReference` and
`NumericalWorkflow.perturbation_budget` as follows:

```python
specification.feedback_policy.CopyFrom(common_pb2.PolicyReference(
    policy_id=common_pb2.PolicyId(value="feedback.us.baseline"),
    revision="1",
    sha256=common_pb2.Sha256Digest(value=hashlib.sha256(policy_bytes).digest()),
))
specification.numerical.perturbation_budget.CopyFrom(research.ResearchJobBudget(
    maximum_steps=1, maximum_input_tokens=0, maximum_output_tokens=0,
    maximum_cost=common_pb2.Money(
        currency_code="USD", amount=common_pb2.ExactDecimal(value="0"),
    ),
    maximum_wall_time=Duration(seconds=20),
))
protocol = specification.numerical.protocol_selection
del protocol.enabled_features[:]
protocol.enabled_features.extend([
    "jobs.envelope.v1", "jobs.kind-input.v1", "jobs.perturbation.v1",
    "jobs.prelease-terminal.v1",
])
protocol.selection_sha256.value = protocol_selection_sha256(protocol)
```

Use the current deployed descriptor/build snapshot and `selected_package =
"loop.v1"`; the four features above are exact and sorted. The existing Discovery
snapshot keeps its own capabilities. Public Run/Discovery RPCs do not accept a
perturbation work item, random state, replacement message, lease or protected
capability. The service derives and reserves that internal job itself.

Perturbation requires exactly one step, zero input/output tokens, zero USD model
cost and positive wall time no greater than 20 seconds. It executes the installed
Python algorithm under a real job lease, using the existing `portfolio.python`
interpreter and frozen research build; no new deployment worker section is
needed. Zero LLM cost does not mean free compute or a server-wide CPU quota.
The same conservative parent reservation rules apply, including no refund on
failure and no clipped child allowance. Three full rounds need up to nine steps
when each proposal and numerical child has a one-step allowance.

Serialize and publish the new canonical template, then publish this wrapper:

```json
{
  "schema": "loop.research-run/v3",
  "id": "research.us.adaptive",
  "revision": "1",
  "specification": {"sha256": "sha256:<v3 template digest>", "byte_size": 2600},
  "numerical_recipe": {"sha256": "sha256:<recipe digest>", "byte_size": 1200},
  "feedback_policy": {"sha256": "sha256:<policy digest>", "byte_size": 400}
}
```

Replace every digest/byte count with actual values. Both policy references must
match the template exactly. Register the wrapper in `runs.plans`, restart the
compatible runtime, and create the operator's binary `PolicyReference` from the
wrapper's ID/revision/digest. The runtime publishes derived feedback, perturbation
work and successful receipt/step evidence into the trusted development artifact
store. These are immutable evidence objects, not new market-data snapshots; do
not delete them when clearing temporary worker directories.

### Execute, inspect and recover

```bash
loopctl run --config /srv/loop/operator/run-client.json --workflow-version 3 start \
  --timeout-seconds 120 --plan /srv/loop/operator/adaptive-plan.binpb --key start.adaptive.001
loopctl run --config /srv/loop/operator/run-client.json --workflow-version 3 step \
  --timeout-seconds 240 --run run.adaptive.001 --revision 1 --key step.adaptive.001
loopctl run --config /srv/loop/operator/run-client.json --workflow-version 3 status \
  --run run.adaptive.001
```

Use the actual registered run ID and the revision returned by the last response.
One command still executes at most one current child and commits one parent
transition. All three operations require version 3. The response schema is
`loop.run-cli/v3`; `research.stage` adds `perturbation`, with a typed Research
child and `perturbation_job_id` instead of a Discovery origin. `run.adaptive`
reports the frozen policy, source count, feedback/context/family/state digests
and cumulative random draws. It does not expose messages, raw source contents,
work, seed, lease or capability. `search_exhausted` is a successful selection
terminal state with the true completed-round count, not a claim that all planned
rounds were evaluated.

Public Discovery `execute`, `resume` and `reconcile` reject v3-owned children,
including a queued child with no model history. Their static templates cannot
replace the frozen feedback invocation. Discovery status exposes only safe
metadata for these children; event reads and authority-reducing stop operations
remain available to the registered Agent. A stop operation grants no new
execution authority. Run-level pause/resume is outside this delivery.

Only registered successful portfolio results from earlier rounds of the same
run can enter feedback. Source result identity, actual files, freshness,
authority and IS applicability are checked again before use. The model receives
at most the last 16 selected sources and 16 KiB of structured feedback, subject
also to its existing context/token ceilings. The complete source history remains
bound to immutable evidence. The full next model invocation is frozen before
any Provider call, and continuation verifies that exact message history.

Unavailable Sharpe remains `unavailable` with its reason; it is never converted
to zero. Coverage rejection and prior empirical failure affect deterministic
filtering and failure memory. Infrastructure faults, stale/corrupt evidence and
unresolved model call outcomes do not become empirical rejection or successful
feedback. An `unknown`/ambiguous model call remains subject to Provider receipt
reconciliation; restarting or changing a command key does not authorize another
possibly chargeable invocation.

After a lost CLI response, reuse the exact version, key and revision. A committed
command receipt returns its original projection without another worker launch.
If the child completed but parent advancement did not commit, recovery rechecks
its original work, lease-bound receipt and step, then applies the saved random
state exactly once in the parent transaction. Missing immutable evidence fails
closed; never recreate it, reset the seed or modify committed state to force
progress. Offline synthetic acceptance establishes execution/accounting behavior,
not improved model quality, investment performance or production data eligibility.

## Disable and recover

Remove the optional `runs` section from the deployment configuration and restart
the service to stop new starts and advancement while retaining owner-scoped
status. An empty `runs.plans` list is invalid. Preserve the database, immutable plans,
child journals, command receipts and audit evidence. Existing Discovery control
commands require the registered Agent identity; changing an operator JSON actor
does not grant that role. Run-level pause/cancel/resume and background scheduling
are subsequent delivery units.

To disable only v3, remove its wrappers from `runs.plans` while retaining the
registered v1/v2 wrappers and restart. If no plans remain, omit `runs` entirely
rather than using an empty array. Retain compatible owner-scoped readers and all
state/evidence. This stops new adaptive starts/advancement without changing old
v0/v2 contracts or rewriting already committed v3 work.

Migrations 0015 and 0016 are additive. The latter adds immutable v2 transition
evidence and fences incompatible writers; installing it does not convert v1
runs. Disable writers before a deployment rollback, keep a
migration-aware binary and preserve research history. Do not deploy an older
writer over the new schema or perform destructive down-migrations. The design
and verification requirements are recorded in
[ADR 0051](../adr/0051-persistent-research-runs.md) and
[ADR 0053](../adr/0053-numerical-run-workflow.md); the v3 extension and its additive
writer fences follow [ADR 0054](../adr/0054-adaptive-research-workflow.md).
Local acceptance commands do not
authorize production migration, dataset purchases or confirmation-set unlocks.
