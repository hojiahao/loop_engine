# Bounded function and method names

Request: review all project-owned code and limit function/method names to at
most three underscore-separated words. `test` counts as one word; private and
dunder delimiters do not. This is a repository convention, not an assertion
about unpublished institutional style guides.

## Change and compatibility

Rename handwritten Python/Rust/Shell callables and TypeScript/JavaScript methods,
functions and named arrows; update imports, exports, string-based selectors,
subprocess test entry points and documentation. Keep detailed test conditions as
comments while shortening identifiers. Examples:

| Before | After |
| --- | --- |
| `test_taf_cap_is_applied_per_modeled_order` | `test_taf_cap` |
| `validate_holdout_backtest_plan_entry_binding` | `validate_plan_binding` |
| `parse_canonical_factor_spec` | `parse_factor_spec` |
| `providerHealth` | `provider_health` |

Public source-level helper imports change in this unreleased workspace; rebuild
all language packages together. No long-name compatibility alias is retained.
Generated Protobuf bindings, message fields, canonical bytes, SQL migrations,
stored audit records and goldens retain their original identities. React's
capitalized `App` and the generated third-party Archify HTML runtime are explicit
exceptions. Production/numerical behavior is not intentionally changed by this
task. Source-dependent provenance becomes stale under the existing integrity
policy; historical receipts are not relabeled.

`just check` now checks Python AST declarations, Rust/Shell declaration patterns
and TypeScript AST declarations, including method signatures/accessors/arrows.
The TypeScript checker uses the already pinned 7.0.2 compiler API; no dependency
is added. Guard tests exercise private names, four-word names, comments, class
methods, interfaces, arrows and wire-field exclusions.

## Acceptance

Commit `9b08a6ecb9bd792aaa01dfc706c491c59078befd` is pushed. GitHub Actions run
[`35079738478`](https://github.com/hojiahao/loop_engine/actions/runs/35079738478)
passes all seven jobs, including clean DaoCloud container and unified
`just check/test/test-isolation/build/doctor`. The local full-test compilation
was stopped on the 1.6 GiB host after prolonged swapping; this exact-commit
remote run supplies the full regression evidence. The publication-time checks
below are retained for traceability.

- Python/Rust/Shell: 3,363 declarations pass the naming gate in the current
  worktree, including the seven new guard functions/methods.
- TypeScript/JavaScript: 343 declarations pass, including guard code and the
  explicit React exception.
- Both guard suites pass: four Python cases and two TypeScript/JavaScript cases.
- Rust formatting and workspace Clippy (`-D warnings`), TypeScript format/lint/
  type checking, Python Ruff/format and both strict-mypy packages pass.
- Protocol regeneration, pinned cross-language wire fixtures, compatibility and
  authority-boundary checks pass without changing protocol/golden artifacts.
- The first combined check reached its final whitespace gate and detected CRLF
  on edited legacy lines. Preserve untouched historical lines and use LF on the
  edited lines; the subsequent `git diff HEAD --check` passes.
- Complete behavioral regression and remote CI: running at publication.

The in-progress Phase 7 action/financing implementation is preserved separately
and is not delivered as part of this naming commit. Its new tests follow the
same rule, including `test_taf_cap`; it retains its own phase-task acceptance.

## Rollback

Revert this naming task as one commit and rebuild every language package. Keep
immutable data, results, trials and audit history; no database down-migration,
artifact rewrite or destructive cleanup is required.
