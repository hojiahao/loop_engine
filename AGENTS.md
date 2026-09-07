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

Read `docs/IMPLEMENTATION_TODO.md` and applicable ADRs before changing a phase.
Do not mark a phase complete until its exit gate, commit, and push succeed.
