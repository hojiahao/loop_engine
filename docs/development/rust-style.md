# Rust naming and review conventions

Loop Engine follows the [Rust Style Guide](https://doc.rust-lang.org/style-guide/)
and [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/). Microsoft's
public [short-name guidance](https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html#names-of-items-are-short-m-short-names)
is an additional design reference, not a claim of compliance with a company's
private engineering standards.

## Names

- Functions, methods, modules, and variables use `snake_case`; types use
  `UpperCamelCase` and constants use `SCREAMING_SNAKE_CASE`.
- Use the shortest name that clearly identifies the behavior in its module.
  Avoid redundant crate or module prefixes and unfamiliar abbreviations.
- Public operations use domain verbs, for example `submit_role`, `get`, and
  `mutate`. Authority, errors, retry, and cancellation semantics belong in their
  API documentation, not in a sentence-length identifier.
- Tests name a behavior or a condition and outcome, for example
  `rejects_missing_inputs` or `changed_input_conflicts`. Multiple assertions
  can verify one invariant; unrelated behaviors should be separate tests.
- Do not introduce an arbitrary identifier-length limit or require one assertion
  per test. Neither rustfmt nor Clippy establishes whether a name or test scope
  is a good design.

## Running a specific test

Test functions are invoked by the test runner, not by production callers. A
substring filter avoids typing a full test identifier:

```bash
./scripts/cargo.sh test --locked --offline -p loopd --test role_submission missing_inputs
```

For an exact name use `-- --exact` with its full test path. Formatting, Clippy,
behavioral tests, real-process concurrency checks, and code review remain
separate mandatory gates.
