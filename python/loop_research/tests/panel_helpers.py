"""Private, invented source captures shared by panel construction tests."""

import csv
import io
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from loop_research.data.fetch_cache import publish, read_cached
from loop_research.panel_builder import build_panel, load_panel_request
from loop_research.panel_models import PanelReceipt, PanelReport, PanelRequest

FIXTURES = Path(__file__).resolve().parents[3] / "fixtures/market/panels"


@dataclass
class Case:
    sources: Path
    output: Path
    request: PanelRequest

    def capture(self) -> dict[str, Any]:
        return json.loads(read_cached(self.sources, self.request.capture))  # type: ignore[no-any-return]

    def replace_capture(self, value: dict[str, Any]) -> None:
        reference = publish(self.sources, json.dumps(value).encode())
        self.request = change(self.request, capture=reference.model_dump())

    def build(self) -> PanelReport:
        return build_panel(self.sources, self.output, self.request)

    def rows(self, report: PanelReport) -> list[dict[str, str]]:
        receipt = PanelReceipt.model_validate_json(read_cached(self.sources, report.receipt))
        return list(csv.DictReader(io.StringIO(read_cached(self.output, receipt.values).decode())))


def change(request: PanelRequest, **values: object) -> PanelRequest:
    return PanelRequest.model_validate_json(
        json.dumps({**request.model_dump(mode="json", by_alias=True), **values})
    )


def make_case(directory: Path) -> Case:
    sources, output = directory / "sources", directory / "output"
    sources.mkdir(mode=0o700)
    output.mkdir(mode=0o700)
    for name in ("source.json", "capture.json"):
        publish(sources, (FIXTURES / name).read_bytes())
    return Case(sources, output, load_panel_request(FIXTURES / "request.json"))
