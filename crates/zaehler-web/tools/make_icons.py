"""Erzeugt die App-Icons (Zeigerinstrument) als PNG und SVG.

Aufruf: uv run --with pillow python crates/zaehler-web/tools/make_icons.py crates/zaehler-web/static
"""

import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ACCENT = (47, 111, 79)
WHITE = (255, 255, 255)
NEEDLE = (255, 196, 61)


def geometry(scale: float):
    """Maße für eine Kantenlänge von 1.0, skaliert um `scale` um die Mitte."""
    c = 0.5
    r = 0.30 * scale
    width = 0.07 * scale
    cy = c + 0.06 * scale
    return c, cy, r, width


def draw(size: int, maskable: bool, rounded: bool) -> Image.Image:
    ss = 4
    n = size * ss
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    if rounded and not maskable:
        d.rounded_rectangle([0, 0, n - 1, n - 1], radius=int(0.22 * n), fill=ACCENT)
    else:
        d.rectangle([0, 0, n, n], fill=ACCENT)
    # maskable: Motiv innerhalb der sicheren Zone (Kreis mit 80 % Durchmesser)
    scale = 0.8 if maskable else 1.0
    c, cy, r, w = geometry(scale)
    box = [(c - r) * n, (cy - r) * n, (c + r) * n, (cy + r) * n]
    d.arc(box, start=150, end=390, fill=WHITE, width=int(w * n))
    # Skalenstriche
    for i in range(5):
        a = math.radians(150 + i * 60)
        r1, r2 = r - w * 1.4, r - w * 2.4
        d.line(
            [
                ((c + r1 * math.cos(a)) * n, (cy + r1 * math.sin(a)) * n),
                ((c + r2 * math.cos(a)) * n, (cy + r2 * math.sin(a)) * n),
            ],
            fill=WHITE,
            width=int(w * 0.45 * n),
        )
    # Zeiger
    a = math.radians(300)
    tip = ((c + (r - w * 1.6) * math.cos(a)) * n, (cy + (r - w * 1.6) * math.sin(a)) * n)
    d.line([(c * n, cy * n), tip], fill=NEEDLE, width=int(w * 0.8 * n))
    hub = w * 0.9
    d.ellipse([(c - hub) * n, (cy - hub) * n, (c + hub) * n, (cy + hub) * n], fill=NEEDLE)
    return img.resize((size, size), Image.LANCZOS)


def svg() -> str:
    c, cy, r, w = geometry(1.0)
    s = 512
    def p(x, y):
        return f"{x * s:.1f} {y * s:.1f}"
    a0, a1 = math.radians(150), math.radians(390)
    start = (c + r * math.cos(a0), cy + r * math.sin(a0))
    end = (c + r * math.cos(a1), cy + r * math.sin(a1))
    ticks = []
    for i in range(5):
        a = math.radians(150 + i * 60)
        r1, r2 = r - w * 1.4, r - w * 2.4
        ticks.append(
            f'<path d="M{p(c + r1 * math.cos(a), cy + r1 * math.sin(a))} '
            f'L{p(c + r2 * math.cos(a), cy + r2 * math.sin(a))}"/>'
        )
    a = math.radians(300)
    tip = (c + (r - w * 1.6) * math.cos(a), cy + (r - w * 1.6) * math.sin(a))
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">\n'
        f'<rect width="512" height="512" rx="{0.22 * s:.0f}" fill="#2f6f4f"/>\n'
        f'<path d="M{p(*start)} A{r * s:.1f} {r * s:.1f} 0 1 1 {p(*end)}" fill="none" '
        f'stroke="#fff" stroke-width="{w * s:.1f}"/>\n'
        f'<g stroke="#fff" stroke-width="{w * 0.45 * s:.1f}">{"".join(ticks)}</g>\n'
        f'<path d="M{p(c, cy)} L{p(*tip)}" stroke="#ffc43d" stroke-width="{w * 0.8 * s:.1f}"/>\n'
        f'<circle cx="{c * s:.1f}" cy="{cy * s:.1f}" r="{w * 0.9 * s:.1f}" fill="#ffc43d"/>\n'
        "</svg>\n"
    )


def main() -> None:
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    draw(192, False, True).save(out / "icon-192.png", optimize=True)
    draw(512, False, True).save(out / "icon-512.png", optimize=True)
    draw(512, True, False).save(out / "icon-maskable-512.png", optimize=True)
    # iOS rundet selbst ab und erwartet ein Bild ohne Transparenz
    draw(180, False, False).convert("RGB").save(out / "apple-touch-icon.png", optimize=True)
    (out / "icon.svg").write_text(svg())


if __name__ == "__main__":
    main()
