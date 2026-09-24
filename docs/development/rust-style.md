# Function naming and Rust review conventions

Loop Engine follows the [Rust Style Guide](https://doc.rust-lang.org/style-guide/)
and [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/). Microsoft's
public [short-name guidance](https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html#names-of-items-are-short-m-short-names)
is an additional design reference, not a claim of compliance with a company's
private engineering standards.

## Names

- In Rust, functions, methods, modules, and variables use `snake_case`; types use
  `UpperCamelCase` and constants use `SCREAMING_SNAKE_CASE`.
- Use the shortest name that clearly identifies the behavior in its module.
  Avoid redundant crate or module prefixes and unfamiliar abbreviations.
- Public operations use domain verbs, for example `submit_role`, `get`, and
  `mutate`. Authority, errors, retry, and cancellation semantics belong in their
  API documentation, not in a sentence-length identifier.
- Tests name a behavior or a condition and outcome, for example
  `rejects_missing_inputs` or `changed_input_conflicts`. Multiple assertions
  can verify one invariant; unrelated behaviors should be separate tests.
- Handwritten functions and methods across Rust, Python, TypeScript/JavaScript
  and Shell use `snake_case` with at most **three words**. This is Loop Engine's
  project convention, not a rule attributed to Rust or any company's private
  standards. Leading private underscores and Python dunder delimiters do not
  count; the `test` prefix does. For example, use `test_taf_cap`,
  `validate_plan_binding` and `_validate_holdout_input`.
- Keep a name meaningful in its module. Preserve the full test condition in a
  nearby comment or docstring; do not replace words with cryptic abbreviations
  or merge unrelated scenarios to satisfy the limit.
- Generated Protobuf bindings retain their generator-defined names. React's
  `App` component remains capitalized so JSX treats it as a component. External
  API/wire fields, immutable SQL migrations and historical research/audit data
  are not function declarations and are not renamed. The standalone HTML diagrams
  retain the third-party Archify viewer generated into them; that vendored runtime
  is not maintained as Loop Engine source. Any future framework-fixed
  function exception needs an exact file/name entry and a documented reason;
  do not use a broad naming exemption for handwritten modules.
- `just check` runs `scripts/check-function-names.py` and the pinned TypeScript
  compiler's AST check. Python uses its standard AST; Rust/Shell declarations
  are checked by line-based declaration patterns. TypeScript method signatures,
  accessors and named arrow functions are included. Imports of external APIs
  are not declarations owned by this project. Compiler checks and behavioral
  tests remain necessary to verify references and behavior.
- Do not require one assertion per test. Neither rustfmt nor Clippy establishes
  whether a name or test scope is a good design.

## Running a specific test

Test functions are invoked by the test runner, not by production callers. A
substring filter avoids typing a full test identifier:

```bash
./scripts/cargo.sh test --locked --offline -p loopd --test role_submission missing_inputs
```

For an exact name use `-- --exact` with its full test path. Formatting, Clippy,
behavioral tests, real-process concurrency checks, and code review remain
separate mandatory gates.
