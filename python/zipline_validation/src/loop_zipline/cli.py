"""Bounded independent accounting entry point with disposable vendor runtime files."""

import argparse
import json
import os
import tempfile
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(prog="loop-zipline")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor")
    execute = commands.add_parser("run")
    execute.add_argument("--input", required=True)
    replay = commands.add_parser("validate")
    replay.add_argument("--receipt", required=True)
    for command in (execute, replay):
        command.add_argument("--store", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="loop-engine-zipline-") as temporary:
        os.environ["ZIPLINE_ROOT"] = temporary
        os.environ["MPLCONFIGDIR"] = temporary
        os.environ["MPLBACKEND"] = "Agg"
        from loop_zipline.artifacts import Deadline
        from loop_zipline.build import describe
        from loop_zipline.workflow import run

        try:
            if args.command == "doctor":
                build = json.loads(describe(Deadline()))
                print(json.dumps({"python": build["python"], "versions": build["versions"]}))
                return
            report = run(
                args.store,
                args.input if args.command == "run" else args.receipt,
                replay=args.command == "validate",
            )
        except (OSError, ValueError, TimeoutError):
            parser.error("independent accounting failed: invalid input, build, artifact or budget")
        except KeyboardInterrupt:
            parser.exit(130, "independent accounting cancelled; preserve immutable evidence\n")
        print(json.dumps(report, separators=(",", ":"), allow_nan=False))
        parser.exit(
            {"accepted": 0, "rejected": 3, "unavailable": 4}[report["artifacts"]["disposition"]]
        )


if __name__ == "__main__":
    main()
