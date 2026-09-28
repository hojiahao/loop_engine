# Invented causal panel input

All security identities, prices and source evidence in this directory are
synthetic. They are not downloaded market data or a performance result.

- `source.json`: original invented source bytes.
- `capture.json`: source-backed security histories and six raw bars for two
  securities over the 2010-01-04 through 2010-01-06 XNYS sessions.
- `request.json`: exact capture identity, raw close selection and evaluation
  interval, including the nontrading start boundary 2010-01-02.

Tests publish the first two files into a private source CAS, run the installed
`panel-build` command and pass its resulting dataset through the authorized
numerical worker. Expected raw closes are 8, 10 and 12 for both securities;
two-observation moving averages have four valid evaluation cells out of six.

Changing fixture bytes requires updating their content references. Run
`tests/test_panel_builder.py` and the Rust manifest/runtime evaluation suite to
verify the actual cross-component handoff. No API credential or data license is
used by this fixture.
