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
    access = commands.add_parser("data-preflight", help="Offline source credential/license checks")
    access.add_argument("config", type=Path)
    access.add_argument("--store", type=Path, required=True)
    access.add_argument("--license", type=Path, action="append", default=[])
    fetch = commands.add_parser("data-fetch", help="Bounded SEC/Alpaca development acquisition")
    fetch.add_argument("config", type=Path)
    fetch.add_argument("--store", type=Path, required=True)
    replay = commands.add_parser("data-replay", help="Offline verification of a cached acquisition")
    replay.add_argument("--store", type=Path, required=True)
    replay.add_argument("--receipt", required=True)
    licensed = commands.add_parser("data-acquire", help="Acquire explicitly licensed source data")
    licensed.add_argument("config", type=Path)
    licensed.add_argument("--license", type=Path, required=True)
    licensed.add_argument("--store", type=Path, required=True)
    licensed_replay = commands.add_parser(
        "data-verify", help="Offline licensed-source receipt replay"
    )
    licensed_replay.add_argument("--store", type=Path, required=True)
    licensed_replay.add_argument("--receipt", required=True)
    snapshot = commands.add_parser("data-snapshot", help="Build immutable source Parquet snapshots")
    snapshot.add_argument("--store", type=Path, required=True)
    snapshot.add_argument("--receipt", action="append", required=True)
    snapshot.add_argument("--start", required=True)
    snapshot.add_argument("--through", required=True)
    validate = commands.add_parser(
        "data-validate", help="Replay source snapshot lineage and Parquet"
    )
    validate.add_argument("--store", type=Path, required=True)
    validate.add_argument("--snapshot", required=True)
    sync = commands.add_parser("data-sync", help="Synchronize an explicit bounded source plan")
    sync.add_argument("plan", type=Path)
    sync.add_argument("--store", type=Path, required=True)
    sync.add_argument("--license", type=Path, action="append", default=[])
    sync.add_argument("--resume")
    query = commands.add_parser("data-query", help="Read-only local point-in-time data query")
    query.add_argument("input", type=Path)
    query.add_argument("--market-at", required=True)
    query.add_argument("--known-at", required=True)
    query.add_argument("--ingested-at", required=True)
    selector = query.add_mutually_exclusive_group()
    selector.add_argument("--security-id")
    selector.add_argument("--ticker")
    query.add_argument("--venue")
    panel = commands.add_parser("panel-build", help="Build causal development OHLCV panels")
    panel.add_argument("request", type=Path)
    panel.add_argument("--sources", type=Path, required=True)
    panel.add_argument("--store", type=Path, required=True)
    panel_check = commands.add_parser("panel-validate", help="Offline causal panel reconstruction")
    panel_check.add_argument("--receipt", required=True)
    panel_check.add_argument("--sources", type=Path, required=True)
    panel_check.add_argument("--store", type=Path, required=True)
    backtest = commands.add_parser("backtest-run", help="Replay a frozen development portfolio")
    backtest.add_argument("request", type=Path)
    backtest_check = commands.add_parser(
        "backtest-validate", help="Reconstruct every immutable portfolio ledger offline"
    )
    backtest_check.add_argument("--receipt", required=True)
    for command in (backtest, backtest_check):
        command.add_argument("--evidence", type=Path, required=True)
        command.add_argument("--view", type=Path, required=True)
        command.add_argument("--store", type=Path, required=True)
    statistics = commands.add_parser(
        "statistics-run", help="Evaluate a verified portfolio and complete trial family"
    )
    statistics.add_argument("request", type=Path)
    statistics_check = commands.add_parser(
        "statistics-validate", help="Read-only statistical evidence reconstruction"
    )
    statistics_check.add_argument("--receipt", required=True)
    for command in (statistics, statistics_check):
        command.add_argument("--evidence", type=Path, required=True)
        command.add_argument("--view", type=Path, required=True)
        command.add_argument("--store", type=Path, required=True)
    alphalens = commands.add_parser(
        "alphalens-prepare", help="Export verified raw inputs for independent statistics"
    )
    alphalens.add_argument("--statistics", required=True)
    for name in ("evidence", "view", "store"):
        alphalens.add_argument("--" + name, type=Path, required=True)
    zipline = commands.add_parser(
        "zipline-prepare", help="Export verified raw inputs for independent accounting"
    )
    zipline.add_argument("--backtest", required=True)
    for name in ("evidence", "view", "store"):
        zipline.add_argument("--" + name, type=Path, required=True)
    binding = commands.add_parser(
        "statistics-bind", help="Prepare a trial binding before freezing its plan policy"
    )
    binding.add_argument("request", type=Path)
    binding.add_argument("--evidence", type=Path, required=True)
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
    elif args.command == "data-preflight":
        from loop_research.data.access import load_access_config, preflight

        try:
            access_report = preflight(
                load_access_config(args.config), args.store, licenses=tuple(args.license)
            )
        except OSError, ValueError:
            parser.error("data access preflight failed: malformed, unsafe or sensitive input")
        except KeyboardInterrupt:
            parser.exit(130, "data access preflight cancelled\n")
        print(access_report.model_dump_json(by_alias=True))
        if not access_report.local_ready:
            parser.exit(3)
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
    elif args.command == "statistics-bind":
        from google.protobuf.message import DecodeError  # type: ignore[import-untyped]

        from loop_research.backtest import load_request
        from loop_research.statistics_workflow import bind_request

        try:
            binding_identity = bind_request(args.evidence, load_request(args.request))
        except OSError, ValueError, DecodeError:
            parser.error("trial binding failed: invalid request or work evidence")
        print(json.dumps({"binding_sha256": binding_identity}))
    elif args.command == "zipline-prepare":
        from loop_research.zipline_inputs import prepare_zipline

        try:
            accounting_input = prepare_zipline(args.evidence, args.view, args.store, args.backtest)
        except OSError, ValueError, TimeoutError:
            parser.error("independent export failed: invalid portfolio, provenance or budget")
        except KeyboardInterrupt:
            parser.exit(130, "independent export cancelled; preserve immutable evidence\n")
        print(accounting_input.model_dump_json())
    elif args.command == "alphalens-prepare":
        from loop_research.alphalens_inputs import prepare_alphalens

        try:
            independent_input = prepare_alphalens(
                args.evidence, args.view, args.store, args.statistics
            )
        except OSError, ValueError, TimeoutError:
            parser.error("independent export failed: invalid statistics, provenance or budget")
        except KeyboardInterrupt:
            parser.exit(130, "independent export cancelled; preserve immutable evidence\n")
        print(independent_input.model_dump_json())
    elif args.command in ("statistics-run", "statistics-validate"):
        from loop_research.statistics_workflow import (
            load_statistics,
            run_statistics,
            validate_statistics,
        )

        try:
            statistics_report = (
                run_statistics(args.evidence, args.view, args.store, load_statistics(args.request))
                if args.command == "statistics-run"
                else validate_statistics(args.evidence, args.view, args.store, args.receipt)
            )
        except OSError, ValueError, TimeoutError:
            parser.error("statistics operation failed: invalid evidence, policy, family or budget")
        except KeyboardInterrupt:
            parser.exit(130, "statistics operation cancelled; preserve immutable input evidence\n")
        print(statistics_report.model_dump_json(by_alias=True))
    elif args.command in ("backtest-run", "backtest-validate"):
        from loop_research.backtest import load_request, run_backtest, validate_backtest

        try:
            backtest_report = (
                run_backtest(args.evidence, args.view, args.store, load_request(args.request))
                if args.command == "backtest-run"
                else validate_backtest(args.evidence, args.view, args.store, args.receipt)
            )
        except OSError, ValueError, TimeoutError:
            parser.error(
                "portfolio operation failed: invalid evidence, policy, accounting or budget"
            )
        except KeyboardInterrupt:
            parser.exit(130, "portfolio operation cancelled; preserve immutable input evidence\n")
        print(backtest_report.model_dump_json(by_alias=True))
    elif args.command in ("panel-build", "panel-validate"):
        from loop_research.panel_builder import build_panel, load_panel_request, validate_panel

        try:
            panel_report = (
                build_panel(args.sources, args.store, load_panel_request(args.request))
                if args.command == "panel-build"
                else validate_panel(args.sources, args.store, args.receipt)
            )
        except OSError, ValueError, TimeoutError:
            parser.error("panel operation failed: invalid source, selection, provenance or budget")
        except KeyboardInterrupt:
            parser.exit(130, "panel operation cancelled; preserve immutable source evidence\n")
        print(panel_report.model_dump_json(by_alias=True))
    elif args.command in ("data-snapshot", "data-validate", "data-sync"):
        from loop_research.data.fetch_http import FetchError
        from loop_research.data.snapshot_models import SnapshotRequest
        from loop_research.data.snapshots import build_snapshot, validate_snapshot
        from loop_research.data.sync import load_sync_plan, synchronize

        try:
            if args.command == "data-sync":
                snapshot_report = asyncio.run(
                    synchronize(
                        args.store,
                        load_sync_plan(args.plan),
                        licenses=tuple(args.license),
                        resume=args.resume,
                        progress=lambda reference, count: print(
                            json.dumps(
                                {
                                    "event": "sync_progress",
                                    "completed_requests": count,
                                    "progress": reference.model_dump(),
                                }
                            ),
                            flush=True,
                        ),
                    )
                )
            elif args.command == "data-validate":
                snapshot_report = validate_snapshot(args.store, args.snapshot)
            else:
                snapshot_request = SnapshotRequest.model_validate_json(
                    json.dumps(
                        {
                            "receipts": sorted(args.receipt),
                            "start": args.start,
                            "through": args.through,
                        }
                    )
                )
                snapshot_report = build_snapshot(args.store, snapshot_request)
        except FetchError as error:
            parser.error(f"source snapshot operation failed: {error.reason}")
        except OSError, ValueError, TimeoutError:
            parser.error(
                "source snapshot operation failed: invalid evidence, input, budget or local IO"
            )
        except KeyboardInterrupt:
            parser.exit(130, "source snapshot operation cancelled; preserve immutable progress\n")
        print(snapshot_report.model_dump_json(by_alias=True))
    elif args.command in ("data-acquire", "data-verify"):
        from loop_research.data.fetch_http import FetchError
        from loop_research.data.licensed_ingestion import (
            fetch_licensed,
            load_config,
            replay_licensed,
        )

        try:
            licensed_report = (
                asyncio.run(fetch_licensed(load_config(args.config), args.store, args.license))
                if args.command == "data-acquire"
                else replay_licensed(args.store, args.receipt)
            )
        except FetchError as error:
            status = f" (HTTP {error.status_code})" if error.status_code is not None else ""
            parser.error(f"licensed data operation failed: {error.reason}{status}")
        except OSError, ValueError:
            parser.error("licensed data operation failed: invalid input or local IO")
        except KeyboardInterrupt:
            parser.exit(130, "licensed data operation cancelled; preserve cached evidence\n")
        print(licensed_report.model_dump_json(by_alias=True))
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
