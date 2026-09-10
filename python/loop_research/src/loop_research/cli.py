"""Research worker command line entry point."""

import argparse
import json
from dataclasses import asdict
from pathlib import Path

from loop_research.health import research_health
from loop_research.nav_diagnostic import correlate_nav_files


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="loop-research")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor")
    correlation = commands.add_parser("nav-correlation", help="Read-only local NAV diagnostics")
    correlation.add_argument("left", type=Path)
    correlation.add_argument("right", type=Path)
    correlation.add_argument("--min-observations", type=int, default=5)
    correlation.add_argument("--calendar", choices=["XNYS"], default=None)
    correlation.add_argument(
        "--cash-flow-adjusted",
        action="store_true",
        required=True,
        help="Assert both inputs already remove external deposits and withdrawals",
    )
    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    if args.command == "doctor":
        print(research_health().model_dump_json())
    elif args.command == "nav-correlation":
        try:
            report = correlate_nav_files(
                args.left,
                args.right,
                min_observations=args.min_observations,
                cash_flow_adjusted=args.cash_flow_adjusted,
                calendar=args.calendar,
            )
        except (OSError, ValueError) as error:
            parser.error(str(error))
        print(json.dumps(asdict(report), allow_nan=False, sort_keys=True))


if __name__ == "__main__":
    main()
