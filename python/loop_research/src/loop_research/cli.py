"""Research worker command line entry point."""

import argparse
import asyncio
import json
from dataclasses import asdict
from pathlib import Path

from loop_research.build_identity import describe_build
from loop_research.health import research_health
from loop_research.nav_diagnostic import correlate_nav_files


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="loop-research")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor")
    manifests = commands.add_parser("build-manifests", help="Capture the installed worker build")
    manifests.add_argument("--store", type=Path, required=True)
    manifests.add_argument(
        "--profile", choices=["perturbation", "evaluation"], default="perturbation"
    )
    fetch = commands.add_parser("data-fetch", help="Bounded SEC/Alpaca development acquisition")
    fetch.add_argument("config", type=Path)
    fetch.add_argument("--store", type=Path, required=True)
    replay = commands.add_parser("data-replay", help="Offline verification of a cached acquisition")
    replay.add_argument("--store", type=Path, required=True)
    replay.add_argument("--receipt", required=True)
    query = commands.add_parser("data-query", help="Read-only local point-in-time data query")
    query.add_argument("input", type=Path)
    query.add_argument("--market-at", required=True)
    query.add_argument("--known-at", required=True)
    query.add_argument("--ingested-at", required=True)
    selector = query.add_mutually_exclusive_group()
    selector.add_argument("--security-id")
    selector.add_argument("--ticker")
    query.add_argument("--venue")
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
    elif args.command == "build-manifests":
        try:
            identity = describe_build(args.store, profile=args.profile)
        except OSError, ValueError:
            parser.error("worker build capture failed; no existing object was overwritten")
        print(json.dumps(asdict(identity), separators=(",", ":")))
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
    elif args.command == "data-query":
        from loop_research.data.diagnostic import query_file
        from loop_research.data.models import PitQuery

        try:
            query = PitQuery(
                market_at=args.market_at,
                known_at=args.known_at,
                ingested_at=args.ingested_at,
                security_id=args.security_id,
                ticker=args.ticker,
                venue=args.venue,
            )
            pit_report = query_file(args.input, query)
        except OSError, ValueError:
            # Never echo record bodies, vendor payloads or private local paths.
            parser.error("PIT query failed: invalid clocks, records, selection or input file")
        print(pit_report.model_dump_json(by_alias=True))
    elif args.command in ("data-fetch", "data-replay"):
        from loop_research.data.fetch_http import FetchError
        from loop_research.data.ingestion import fetch_data, load_fetch_config, replay_data

        try:
            fetch_report = (
                asyncio.run(fetch_data(load_fetch_config(args.config), args.store))
                if args.command == "data-fetch"
                else replay_data(args.store, args.receipt)
            )
        except FetchError as error:
            status = f" (HTTP {error.status_code})" if error.status_code is not None else ""
            parser.error(f"development data operation failed: {error.reason}{status}")
        except OSError, ValueError:
            parser.error("development data operation failed: invalid input or local IO")
        except KeyboardInterrupt:
            parser.exit(130, "development data operation cancelled; preserve cached evidence\n")
        print(fetch_report.model_dump_json(by_alias=True))


if __name__ == "__main__":
    main()
