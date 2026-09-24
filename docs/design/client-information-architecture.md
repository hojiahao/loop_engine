# Client information architecture

The validated interactive system map is available at
[`../diagrams/loop-engine-clients.architecture.html`](../diagrams/loop-engine-clients.architecture.html).
Its typed source and browser evidence are retained in the same directory.

## Product shape

Loop Engine has three clients over the same `loopd` API and authorization
model. It is a research operations system, not a brokerage order-entry terminal
or a market-data encyclopedia.

| Client | Primary job | Design reference point |
| --- | --- | --- |
| React Web | Research analysis, comparison, governance, audit | Dense institutional research console |
| Ratatui TUI | SSH operation, loop supervision, recovery, approval | Agent-oriented terminal workspace |
| `loopctl` | Automation, CI, exact administration | Stable scriptable command surface |

## Web workspace

```text
┌ Loop Engine · US Equities · DEV · Data 2026-08-31 · Spend $18.42 · hojiahao ┐
├──────────────┬───────────────────────────────────────────────────────────────┤
│ Overview     │ Research runs                               New run   Pause   │
│ Runs      12 │ ┌────────┬───────────┬────────┬────────┬──────────┬─────────┐ │
│ Data       4 │ │ Run    │ Stage     │ Model  │ Budget │ Progress │ Status  │ │
│ Factors   83 │ │ R-1842 │ IS review │ Claude │ $3/$10 │ 18/40    │ Running │ │
│ Backtests  7 │ │ R-1841 │ Reconcile │ GPT    │ $7/$10 │ 40/40    │ Failed  │ │
│ Providers 16 │ └────────┴───────────┴────────┴────────┴──────────┴─────────┘ │
│ Holdouts  🔒 │                                                               │
│ Audit        │ Evidence and events                                           │
│ Settings     │ 14:32  AST canonicalized    sha256:91ab...                    │
│              │ 14:33  Duplicate filter     passed                            │
│              │ 14:34  IS backtest          Rank IC 0.031 · NW t 2.41         │
│              │ 14:35  Checker review       pending                           │
├──────────────┴───────────────────────────────────────────────────────────────┤
│ Data healthy · Holdouts locked · 2 provider warnings · Audit append healthy │
└───────────────────────────────────────────────────────────────────────────────┘
```

The factor workspace uses tabs for AST and lineage, statistics, primary versus
independent backtest reconciliation, exposures, and audit events. Comparison
tables keep identifiers and decision state pinned while numeric columns scroll.

Holdout unlock, forced readmission, budget elevation, and secret binding live in
separate approval flows. They require a named actor, reason, evidence reference,
and confirmation of the exact immutable target.

## TUI workspace

```text
┌ Loop Engine TUI ─ run R-1842 ─ RUNNING ─ 18/40 steps ─ $3.18/$10.00 ┐
│ Loop              │ Current step                                    │
│  ✓ propose AST    │ checker.semantic_review                         │
│  ✓ canonicalize   │ Provider  anthropic / pinned-model-id           │
│  ✓ failed memory  │ Tool      review_factor                         │
│  ✓ IS evaluate    │ Started   14:35:12   Remaining  00:01:43        │
│  → checker        ├─────────────────────────────────────────────────┤
│  · decision       │ Event stream                                    │
│  · persist        │ 14:35:12 request validated                      │
│                   │ 14:35:13 tool capability granted                │
│ [p] pause         │ 14:35:16 structured block: thesis               │
│ [c] cancel        │ 14:35:18 stream active                          │
│ [a] audit         │                                                 │
└───────────────────┴─────────────────────────────────────────────────┘
```

TUI interaction is keyboard-first and does not assume mouse or rich graphics.
It exposes the same termination reasons, approvals, and audit identifiers as the
Web interface.

## Mobile policy

Mobile supports monitoring, acknowledge/deny approval, pause/cancel, provider
health, and concise audit inspection. Wide factor comparison, time-series
diagnostics, and reconciliation ledgers remain desktop tasks; mobile links to a
specific immutable report instead of shrinking unreadable tables.

## Visual and accessibility constraints

- Quiet, neutral surfaces with status color reserved for actionable state.
- No marketing hero, decorative cards, gradient decoration, or fake live data.
- Tabular numerics use aligned digits; IDs and hashes are copyable and never
  truncated without an accessible full value.
- All actions work by keyboard, retain visible focus, and expose text status in
  addition to color.
- Dense desktop views progressively disclose detail and never nest cards.
