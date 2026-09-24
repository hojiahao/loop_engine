"""Bounded administrative entry point, with disposable third-party plot caches."""

import argparse
import importlib.metadata
import json
import os
import platform
import tempfile
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(prog="loop-alphalens")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor")
    execute = commands.add_parser("run")
    execute.add_argument("--input", required=True)
    replay = commands.add_parser("validate")
    replay.add_argument("--receipt", required=True)
    for command in (execute, replay):
        command.add_argument("--store", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="loop-engine-alphalens-") as cache:
        os.environ["MPLCONFIGDIR"] = cache
        os.environ["MPLBACKEND"] = "Agg"
        from loop_alphalens.artifacts import Deadline
        from loop_alphalens.build import describe
        from loop_alphalens.workflow import run

        try:
            if args.command == "doctor":
                describe(Deadline())
                print(
                    json.dumps(
                        {
                            "python": platform.python_version(),
                            "alphalens": importlib.metadata.version("alphalens-reloaded"),
                            "pandas": importlib.metadata.version("pandas"),
                        }
                    )
                )
                return
            report = run(
                args.store,
                args.input if args.command == "run" else args.receipt,
                replay=args.command == "validate",
            )
        except OSError, ValueError, TimeoutError:
            parser.error("independent validation failed: invalid input, build, artifact or budget")
        except KeyboardInterrupt:
            parser.exit(130, "independent validation cancelled; retain immutable evidence\n")
        print(json.dumps(report, separators=(",", ":"), allow_nan=False))
        parser.exit(
            {"accepted": 0, "rejected": 3, "unavailable": 4}[report["artifacts"]["disposition"]]
        )


if __name__ == "__main__":
    main()
