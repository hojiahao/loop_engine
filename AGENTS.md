# Loop Engine contributor contract

## Commands

Run `./scripts/bootstrap.sh` once on a supported clean host. Afterwards use:

```bash
just check
just test
just build
just doctor
```

## Commit messages

- Write all new Git commit subjects and bodies in Simplified Chinese.
- Preserve code identifiers, commands, paths, and version strings as written.
- Do not rewrite published commits solely to change their message language.

## Architecture boundaries

- `crates/loopd` owns persistent orchestration; it must not implement provider
  protocols or numerical research logic.
- `apps/providerd` owns provider transports; it must not access research state
  or holdout storage.
- `python/loop_research` owns data and numerical research; it must not contain
  provider-specific model routing.
- Large datasets remain immutable artifacts and never cross service RPCs.
- Discovery code must never receive a holdout capability.
- Infrastructure failures fail closed and remain distinct from factor rejection.

## Engineering quality gates

- Follow the official Rust Style Guide and Rust API Guidelines where applicable;
  `cargo fmt --check` and Clippy with `-D warnings` are mandatory, not substitutes
  for behavioral tests. Do not claim compliance with unpublished company rules.
- Prefer concise names within their module context. Keep each test focused on
  one behavior; split unrelated scenarios instead of joining them in a sentence.
  Naming and review guidance: `docs/development/rust-style.md`.
- Handwritten `loopd` code forbids unsafe code. New storage APIs deny missing
  documentation; document authority boundaries, errors, replay semantics, and
  cancellation behavior. Never use panic for ordinary invalid input or outages.
- Keep transport authentication separate from caller-supplied actor metadata.
  Default-deny unresolved references and unavailable protocol capabilities.
- Keep state changes, immutable receipts, and audit appends transactional.
  Use revisions and lease fencing; all lock waits, retries, and scans are bounded.
- Concurrency claims require independent OS processes, not only async tasks or
  two pools. Exercise 2/4/8 writers and kill/restart before and after commit.
- Test negative paths, rollback, corruption, deadlines, and clock regression.
  Preserve numerical goldens and cross-language contract tests across phases.
- Keep commits reviewable and document remaining gates honestly. Formatting,
  coverage percentages, or passing mocks alone cannot close an implementation phase.

Read `docs/IMPLEMENTATION_TODO.md` and applicable ADRs before changing a phase.
Do not mark a phase complete until its exit gate, commit, and push succeed.
