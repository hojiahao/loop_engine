# Validated architecture artifacts

## Client and runtime architecture

- Specification: `loop-engine-clients.architecture.json`
- Interactive artifact: `loop-engine-clients.architecture.html`
- Diagram type: `architecture`
- Archify specification SHA-256:
  `4c62b805c86529310e8061d5e972eb2cd148de9b5a64dfae3035609f712acbb1`
- Artifact SHA-256:
  `fb2ef30e1f314063655075c6e48eb72544470f01d9abba980f46ba1d146c70e1`
- Deterministic validation: 9/9 showcase checks, 0 errors, 0 warnings
- Automated browser evidence: passed
- Perceptual review: passed for light and dark captures at 1440x900 and
  2048x1320
- Final-candidate correction rounds: 0

The automated browser receipt covers containment and viewer behavior at
1440x900, 1600x1000, 1920x1080, and 2048x1320. PNG files are evidence sidecars,
not hand-edited design assets.

The browser evidence was generated in the following immutable image, pulled
through DaoCloud:

```text
m.daocloud.io/mcr.microsoft.com/playwright:v1.62.1-noble@sha256:dcc5531e97840b9b5e794f2814476b21571c5124a3fca2267d73041f56e7580e
```

`deliver` proves deterministic artifact validation, the visual-check receipt
proves bounded browser measurements, and the recorded image inspection is the
perceptual review. These claims are intentionally separate.
