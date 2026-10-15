"""Sammelt Trace-Tags aus dem Rust-Code und erzeugt daraus sphinx-needs-Objekte.

Konvention im Code (direkt über dem Element bzw. dem Test). In Rust mit ``//``,
in YAML, Dockerfile, Helm-Templates und Python mit ``#``:

    // @impl SPEC_CONS_DELTA, SPEC_CONS_DEVICE_SWAP
    pub fn intervals(...)

    // @test SPEC_CONS_DELTA
    #[test]
    fn delta_handles_rollover() { ... }

Pro Tag entsteht ein ``impl``- bzw. ``test``-Need mit Link (``implements`` /
``verifies``) auf die genannten IDs. Unbekannte IDs erzeugen beim Build eine
Warnung für tote Links; mit ``-W`` bricht die CI ab.

Die erzeugte Datei (``codetrace_output``) ist generiert und in ``.gitignore``.
"""

from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass
from pathlib import Path

from sphinx.application import Sphinx
from sphinx.config import Config
from sphinx.util import logging

logger = logging.getLogger(__name__)

TAG_RE = re.compile(r"^\s*(?P<prefix>//|#)\s*@(?P<kind>impl|test)\s+(?P<ids>[A-Z0-9_]+(?:\s*,\s*[A-Z0-9_]+)*)\s*$")
ITEM_RE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?"
    r"(?P<what>fn|struct|enum|trait|impl|mod)\b\s*(?:<[^>]*>\s*)?(?P<name>[A-Za-z_][A-Za-z0-9_:]*)"
)
PY_ITEM_RE = re.compile(r"^\s*(?:async\s+)?(?P<what>def|class)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
LOOKAHEAD = 8
SUFFIXES = {".rs", ".py", ".yaml", ".yml", ".tpl", ".toml"}
NAMES = {"Dockerfile"}

LINK = {"impl": "implements", "test": "verifies"}
TITLE = {"impl": "Implementierung", "test": "Test"}


@dataclass
class Tag:
    kind: str
    targets: list[str]
    path: str
    line: int
    symbol: str


def _module_path(rel: Path) -> str:
    """crates/zaehler-domain/src/consumption.rs -> zaehler_domain::consumption

    Nicht-Rust-Dateien werden über ihren Pfad benannt.
    """
    if rel.suffix != ".rs" or rel.parts[0] != "crates":
        return rel.as_posix()
    parts = rel.parts
    crate = parts[1].replace("-", "_") if len(parts) > 1 else "?"
    mod = [p for p in parts[3:]] if len(parts) > 3 else []
    if mod:
        mod[-1] = mod[-1].removesuffix(".rs")
        if mod[-1] in ("lib", "main", "mod"):
            mod.pop()
    return "::".join([crate, *mod])


def scan(roots: list[Path], base: Path) -> list[Tag]:
    tags: list[Tag] = []
    for root in roots:
        files = [root] if root.is_file() else sorted(root.rglob("*"))
        for file in files:
            if not file.is_file() or "target" in file.parts or "__pycache__" in file.parts:
                continue
            if file.suffix not in SUFFIXES and file.name not in NAMES:
                continue
            lines = file.read_text(encoding="utf-8").splitlines()
            rel = file.resolve().relative_to(base)
            for i, line in enumerate(lines):
                m = TAG_RE.match(line)
                # Rust: nur `//`; sonst nur `#` (verhindert Treffer in Docstrings/Beispielen)
                if not m or (m.group("prefix") == "//") != (file.suffix == ".rs"):
                    continue
                item_re = ITEM_RE if file.suffix == ".rs" else PY_ITEM_RE if file.suffix == ".py" else None
                symbol = None
                for follow in lines[i + 1 : i + 1 + LOOKAHEAD] if item_re else []:
                    item = item_re.match(follow)
                    if item:
                        symbol = item.group("name")
                        break
                module = _module_path(rel)
                tags.append(
                    Tag(
                        kind=m.group("kind"),
                        targets=[t.strip() for t in m.group("ids").split(",")],
                        path=rel.as_posix(),
                        line=i + 1,
                        symbol=f"{module}::{symbol}" if symbol else module,
                    )
                )
    return tags


def render(tags: list[Tag]) -> str:
    out = [
        "# Code-Referenzen",
        "",
        "```{note}",
        "Diese Seite wird bei jedem Build aus den `// @impl`- und `// @test`-Kommentaren",
        "im Rust-Code erzeugt (`docs/_ext/codetrace.py`). Nicht von Hand bearbeiten.",
        "```",
        "",
    ]
    seen: dict[str, int] = {}
    for kind in ("impl", "test"):
        out += [f"## {TITLE[kind]}en" if kind == "impl" else "## Tests", ""]
        for tag in (t for t in tags if t.kind == kind):
            digest = hashlib.sha1(f"{tag.kind}:{tag.symbol}:{','.join(tag.targets)}".encode()).hexdigest()[:8].upper()
            need_id = f"{kind.upper()}_{digest}"
            seen[need_id] = seen.get(need_id, 0) + 1
            if seen[need_id] > 1:
                need_id = f"{need_id}_{seen[need_id]}"
            out += [
                f"```{{{kind}}} {tag.symbol}",
                f":id: {need_id}",
                f":{LINK[kind]}: {', '.join(tag.targets)}",
                ":status: implemented",
                f":source: {tag.path}:{tag.line}",
                "",
                f"`{tag.path}` Zeile {tag.line}",
                "```",
                "",
            ]
    return "\n".join(out)


# @impl SPEC_DOC_TRACING
def generate(app: Sphinx, config: Config) -> None:
    srcdir = Path(app.srcdir)
    base = (srcdir / "..").resolve()
    roots = [(srcdir / r).resolve() for r in config.codetrace_roots]
    tags = scan(roots, base)
    target = srcdir / config.codetrace_output
    content = render(tags)
    # Nur schreiben, wenn sich etwas geändert hat (inkrementelle Builds).
    if not target.exists() or target.read_text(encoding="utf-8") != content:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
    logger.info("codetrace: %d Trace-Tags aus %d Wurzel(n)", len(tags), len(roots))


def setup(app: Sphinx) -> dict[str, object]:
    app.add_config_value("codetrace_roots", ["../crates"], "env", types=(list,))
    app.add_config_value("codetrace_output", "nachverfolgung/code.md", "env")
    app.connect("config-inited", generate)
    return {"version": "0.1", "parallel_read_safe": True, "parallel_write_safe": True}
