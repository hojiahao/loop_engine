"""Write a private connection reference without a command-line password."""

import argparse
import getpass
import os
from pathlib import Path
from urllib.parse import quote, urlencode


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True)
    parser.add_argument("--port", type=int, default=5432)
    parser.add_argument("--database", default="loop_engine")
    parser.add_argument("--user", default="loop_engine_app")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.port <= 65535 or any(c in args.host for c in "/@?#\r\n"):
        parser.error("invalid host or port")
    password = getpass.getpass("Database password: ")
    if not password or password != getpass.getpass("Confirm password: "):
        parser.error("passwords are empty or do not match")
    host = f"[{args.host}]" if ":" in args.host else args.host
    url = (
        f"postgresql://{quote(args.user, safe='')}:{quote(password, safe='')}"
        f"@{host}:{args.port}/{quote(args.database, safe='')}?"
        + urlencode({"sslmode": "require"})
    )
    args.output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(url + "\n")
        output.flush()
        os.fsync(output.fileno())
    print("Private database connection file created; password not displayed.")


if __name__ == "__main__":
    main()
