"""Used fixture package for slim integration tests."""

from __future__ import annotations

import importlib.resources as resources
import json
from typing import Any, Dict


def greet() -> str:
    """Return a friendly greeting used by the consumer project."""

    return "hello from used_pkg"


def load_config() -> Dict[str, Any]:
    """Load JSON configuration bundled with the package resources."""

    with resources.open_text("used_pkg.resources", "config.json", encoding="utf-8") as handle:
        return json.load(handle)


def load_template(name: str = "welcome.txt") -> str:
    """Read a text template stored under the resources/templates directory."""

    with resources.open_text("used_pkg.resources.templates", name, encoding="utf-8") as handle:
        return handle.read()
