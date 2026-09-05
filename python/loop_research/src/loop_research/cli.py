"""Research worker command line entry point."""

import argparse

from loop_research.health import research_health


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="loop-research")
    parser.add_argument("command", choices=["doctor"])
    return parser


def main() -> None:
    args = build_parser().parse_args()
    if args.command == "doctor":
        print(research_health().model_dump_json())


if __name__ == "__main__":
    main()
