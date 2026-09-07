set shell := ["bash", "-euo", "pipefail", "-c"]

bootstrap:
    ./scripts/bootstrap.sh

proto-generate:
    ./scripts/proto-generate.sh --write

proto-check:
    ./scripts/proto-check.sh

check:
    ./scripts/check.sh

test:
    ./scripts/test.sh

build:
    ./scripts/build.sh

doctor:
    ./scripts/cargo.sh run --locked --offline --quiet -p loopctl -- doctor
    ./scripts/uv.sh sync --all-packages --all-groups --locked --offline
    ./scripts/verify-python-environment.sh
    ./scripts/uv-research.sh run --locked --offline --no-sync loop-research doctor
    ./scripts/pnpm.sh --filter @loop-engine/providerd typecheck

container-gate:
    ./scripts/container-gate.sh
