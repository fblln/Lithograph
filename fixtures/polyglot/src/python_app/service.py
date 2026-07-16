"""Service layer used by the polyglot fixture."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


class RouteService:
    """Loads route metadata and delegates expensive work to the Rust worker."""

    def __init__(self, config_path: Path) -> None:
        self.config_path = config_path
        self.worker_path = os.environ.get("RIDGELINE_WORKER", "target/debug/worker")

    def load_settings(self) -> dict[str, object]:
        settings_path = self.config_path / "schema.json"
        return json.loads(settings_path.read_text(encoding="utf-8"))

    def bake_route(self, route_file: Path) -> str:
        # WHY: the worker runs out of process because baking is CPU bound and
        # would otherwise block the request thread.
        return run_worker(self.worker_path, route_file)


def run_worker(worker_path: str, route_file: Path) -> str:
    """Run the Rust worker and return its output."""

    completed = subprocess.run(
        [worker_path, "--route", str(route_file)],
        check=True,
        capture_output=True,
        text=True,
    )
    return completed.stdout

