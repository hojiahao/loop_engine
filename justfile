set shell := ["bash", "-euo", "pipefail", "-c"]

bootstrap:
    ./scripts/bootstrap.sh

check:
    ./scripts/check.sh

test:
    ./scripts/test.sh

build:
    ./scripts/build.sh

doctor:
    ./scripts/cargo.sh run --locked --offline --quiet -p loopctl -- doctor
    ./scripts/uv-research.sh run --locked --offline loop-research doctor
    ./scripts/pnpm.sh --filter @loop-engine/providerd typecheck
