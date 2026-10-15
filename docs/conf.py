"""Sphinx-Konfiguration der Zähler-Dokumentation.

Build:   uv run --project docs sphinx-build -W --keep-going -b html docs docs/_build/html
Live:    uv run --project docs sphinx-autobuild docs docs/_build/html
"""

from __future__ import annotations

import shutil
import sys
from datetime import date
from pathlib import Path

DOCS = Path(__file__).parent
sys.path.insert(0, str(DOCS / "_ext"))

project = "Zähler"
author = "Tom Irgang"
copyright = f"{date.today().year}, {author}"
language = "de"

extensions = [
    "myst_parser",
    "sphinx_needs",
    "sphinxcontrib.mermaid",
    "codetrace",
]

myst_enable_extensions = ["colon_fence", "deflist", "fieldlist", "tasklist"]
myst_heading_anchors = 3

exclude_patterns = ["_build", ".venv", "README.md"]

html_theme = "furo"
html_title = "Zähler-Dokumentation"
html_static_path = ["_static"]
html_css_files = ["custom.css"]

# --- Requirements-Tracing (sphinx-needs) -----------------------------------

needs_id_required = True
needs_id_regex = r"^(REQ|SPEC|ADR|IMPL|TEST|WP)_[A-Z0-9_]+$"
needs_title_optional = False

needs_types = [
    {"directive": "req", "title": "Anforderung", "prefix": "REQ_", "color": "#BFD8D2", "style": "node"},
    {"directive": "spec", "title": "Spezifikation", "prefix": "SPEC_", "color": "#FEDCD2", "style": "node"},
    {"directive": "adr", "title": "Architekturentscheidung", "prefix": "ADR_", "color": "#DCB239", "style": "node"},
    {"directive": "impl", "title": "Implementierung", "prefix": "IMPL_", "color": "#DF744A", "style": "node"},
    {"directive": "test", "title": "Test", "prefix": "TEST_", "color": "#9FD3C7", "style": "node"},
    {"directive": "wp", "title": "Arbeitspaket", "prefix": "WP_", "color": "#C9C9F5", "style": "node"},
]

needs_links = {
    "satisfies": {
        "description": "Spezifikation erfüllt Anforderung",
        "outgoing": "erfüllt",
        "incoming": "erfüllt durch",
        "color": "#2f6f4f",
    },
    "implements": {
        "description": "Code implementiert Spezifikation",
        "outgoing": "implementiert",
        "incoming": "implementiert durch",
        "color": "#DF744A",
    },
    "verifies": {
        "description": "Test verifiziert Spezifikation/Anforderung",
        "outgoing": "verifiziert",
        "incoming": "verifiziert durch",
        "color": "#1f77b4",
        "style": "dashed",
    },
    "covers": {
        "description": "Arbeitspaket setzt Anforderung/Spezifikation um",
        "outgoing": "deckt ab",
        "incoming": "abgedeckt durch",
        "color": "#6a5acd",
    },
    "depends": {
        "description": "Arbeitspaket setzt anderes Arbeitspaket voraus",
        "outgoing": "setzt voraus",
        "incoming": "Voraussetzung für",
        "color": "#888888",
        "style": "dashed",
    },
    "decides": {
        "description": "Entscheidung betrifft Anforderung/Spezifikation",
        "outgoing": "betrifft",
        "incoming": "entschieden in",
        "color": "#DCB239",
        "style": "dotted",
    },
}

needs_fields = {
    "priority": {
        "description": "MoSCoW-Priorität",
        "nullable": True,
        "schema": {"type": "string", "enum": ["must", "should", "could", "wont"]},
    },
    "release": {
        "description": "Geplanter Meilenstein",
        "nullable": True,
        "schema": {"type": "string", "enum": ["MVP", "v1", "später"]},
    },
    "source": {"description": "Fundstelle im Quellcode", "nullable": True},
    "milestone": {
        "description": "Meilenstein innerhalb einer Phase (Arbeitspakete)",
        "nullable": True,
        "schema": {"type": "string", "enum": ["M1", "M2", "M3", "M4"]},
    },
    "status": {
        "description": "Lebenszyklus (req/spec: draft→accepted→implemented, adr: proposed→accepted→superseded)",
        "nullable": True,
        "schema": {
            "type": "string",
            "enum": ["draft", "accepted", "implemented", "proposed", "superseded", "deprecated", "open", "in_progress", "done"],
        },
    },
}


needs_table_style = "DATATABLES"
needs_table_columns = "ID;TITLE;STATUS;TYPE"
needs_build_json = True
needs_flow_engine = "graphviz"

# needflow benötigt Graphviz (`dot`). Ohne Graphviz wird die Graph-Seite durch
# einen Hinweis ersetzt, damit lokale Builds ohne das Systempaket durchlaufen
# (die CI installiert Graphviz und rendert die Graphen).
HAS_GRAPHVIZ = shutil.which("dot") is not None


def _without_graphviz(app, docname, source):
    if docname == "nachverfolgung/graph":
        source[0] = (
            "# Trace-Graph\n\n```{note}\nGraphviz (`dot`) ist lokal nicht installiert; "
            "die Graphen werden im CI-Build erzeugt.\n```\n"
        )


def setup(app):
    if not HAS_GRAPHVIZ:
        app.connect("source-read", _without_graphviz)


# --- Code-Tracing (docs/_ext/codetrace.py) ---------------------------------

codetrace_roots = ["../crates", "../deploy", "../Dockerfile", "../compose.yaml", "../compose.postgres.yaml", "../clippy.toml", "_ext"]
codetrace_output = "nachverfolgung/code.md"
