#!/usr/bin/env python3
"""SENEC-Wochenexporte als tägliche Zählerstände in Zähler importieren.

SENEC liefert je Kalenderwoche eine CSV mit Leistungswerten (kW) in etwa
5-Minuten-Schritten. Das Skript integriert die Leistung (Trapezregel) zu
Energie und schreibt je Tag einen Zählerstand um 00:00 Uhr (Zeitzone des
Haushalts) über `POST /api/v1/readings`:

    Netzbezug        -> Hausanschluss, Bezugsregister    (Standard: "Strom")
    Netzeinspeisung  -> Hausanschluss, Einspeiseregister (Standard: "Strom")
    Stromerzeugung   -> Erzeugungszähler                 (Standard: "PV Erzeugung")
    Akkubeladung/-entnahme -> optional ein Speicherzähler (--battery-meter)

Kalibrierung: Zwischen zwei eigenen Ablesungen (nicht über die API) werden
die Stände so skaliert, dass sie beide genau treffen; die SENEC-Daten
bestimmen nur die Verteilung. Nach der letzten bzw. vor der ersten eigenen
Ablesung wird die SENEC-Energie addiert bzw. abgezogen. Ohne eigene Ablesung
zählt der Stand ab dem Beginn der SENEC-Daten von 0.

Die API speichert je Register, Tag und Token höchstens einen Stand; ein
erneuter Import derselben Woche ersetzt die Werte. Stände vor dem Einbau des
Geräts lehnt die API ab (`reading.before_installation`).

Aufruf:
    python3 scripts/senec_import.py 2026 41 --dry-run
    python3 scripts/senec_import.py 2026 41
    python3 scripts/senec_import.py --file .../S...-week-41-2026.csv
    python3 scripts/senec_import.py --all --quiet

Token: Umgebungsvariable ZAEHLER_API oder Eintrag `ZAEHLER_API=...` in .env
(Rolle Mitglied genügt).
"""

from __future__ import annotations

import argparse
import bisect
import csv
import glob
import json
import os
import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime, time, timedelta, timezone
from decimal import ROUND_HALF_UP, Decimal
from zoneinfo import ZoneInfo

DEFAULT_DIR = "/run/media/tom/FrameData/tom/Nextcloud/Zaehler/Senec"
GRID_IMPORT = "Netzbezug [kW]"
GRID_EXPORT = "Netzeinspeisung [kW]"
GENERATION = "Stromerzeugung [kW]"
BATTERY_CHARGE = "Akkubeladung [kW]"
BATTERY_DISCHARGE = "Akkuentnahme [kW]"
POWER_COLUMNS = [
    GRID_IMPORT,
    GRID_EXPORT,
    "Stromverbrauch [kW]",
    BATTERY_CHARGE,
    BATTERY_DISCHARGE,
    GENERATION,
]
# Längere Lücken werden nicht überbrückt (keine Energie in der Lücke)
MAX_GAP = timedelta(minutes=30)
FILE_RE = re.compile(r"week-(\d+)-(\d{4})\.csv$")


# --- SENEC-Daten -----------------------------------------------------------


@dataclass
class Sample:
    at: datetime  # UTC
    power: dict[str, float]  # kW je Spalte


def read_csv(path: str, tz: ZoneInfo) -> list[Sample]:
    """Eine Wochendatei; Platzhalterzeilen ohne Leistung werden verworfen."""
    out = []
    with open(path, encoding="utf-8-sig", newline="") as f:
        for row in csv.DictReader(f, delimiter=";"):
            power = {c: float((row.get(c) or "0").replace(",", ".")) for c in POWER_COLUMNS}
            if not any(power.values()):
                continue
            local = datetime.strptime(row["Uhrzeit"], "%d.%m.%Y %H:%M:%S")
            out.append(Sample(local.replace(tzinfo=tz).astimezone(timezone.utc), power))
    return out


class Energy:
    """Kumulierte Energie (kWh) je Spalte über alle geladenen Dateien."""

    def __init__(self, samples: list[Sample]):
        samples = sorted({s.at: s for s in samples}.values(), key=lambda s: s.at)
        self.times = [s.at for s in samples]
        self.cum: dict[str, list[float]] = {}
        for col in POWER_COLUMNS:
            acc, values = 0.0, [0.0]
            for a, b in zip(samples, samples[1:]):
                dt = b.at - a.at
                if dt <= MAX_GAP:
                    acc += (a.power[col] + b.power[col]) / 2 * dt.total_seconds() / 3600
                values.append(acc)
            self.cum[col] = values

    @property
    def start(self) -> datetime:
        return self.times[0]

    @property
    def end(self) -> datetime:
        return self.times[-1]

    def at(self, col: str, t: datetime) -> float:
        """Kumulierte Energie bis `t`, zwischen zwei Messpunkten linear."""
        values = self.cum[col]
        if t <= self.times[0]:
            return 0.0
        if t >= self.times[-1]:
            return values[-1]
        i = bisect.bisect_right(self.times, t)
        t0, t1 = self.times[i - 1], self.times[i]
        if t1 - t0 > MAX_GAP:
            return values[i - 1]
        share = (t - t0) / (t1 - t0)
        return values[i - 1] + (values[i] - values[i - 1]) * share

    def between(self, col: str, a: datetime, b: datetime) -> float:
        return self.at(col, b) - self.at(col, a)


def week_file(directory: str, year: int, week: int) -> str:
    matches = [
        p
        for p in glob.glob(os.path.join(directory, "*.csv"))
        if (m := FILE_RE.search(p)) and (int(m.group(2)), int(m.group(1))) == (year, week)
    ]
    if not matches:
        sys.exit(f"Keine Datei für Woche {week}/{year} in {directory}")
    return matches[0]


# --- API -------------------------------------------------------------------


class Api:
    def __init__(self, url: str, token: str):
        self.base = url.rstrip("/") + "/api/v1"
        self.token = token

    def call(self, method: str, path: str, body: object | None = None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(
            self.base + path,
            data=data,
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                raw = r.read()
                return json.loads(raw) if raw else None
        except urllib.error.HTTPError as e:
            sys.exit(f"{method} {path}: HTTP {e.code} {e.read().decode(errors='replace')}")
        except urllib.error.URLError as e:
            sys.exit(f"Zähler nicht erreichbar unter {self.base}: {e.reason}")

    def readings(self, meter_id: str) -> list[dict]:
        out, cursor = [], None
        while True:
            q = f"?limit=1000&cursor={cursor}" if cursor else "?limit=1000"
            page = self.call("GET", f"/meters/{meter_id}/readings{q}")
            out.extend(page["items"])
            cursor = page.get("next_cursor")
            if not cursor:
                return out


def load_token(env_file: str | None) -> str:
    if token := os.environ.get("ZAEHLER_API"):
        return token
    for path in [env_file, ".env", os.path.join(os.path.dirname(__file__), "..", ".env")]:
        if path and os.path.exists(path):
            for line in open(path, encoding="utf-8"):
                key, _, value = line.strip().partition("=")
                if key == "ZAEHLER_API":
                    return value.strip().strip("'\"")
    sys.exit("Kein Token: ZAEHLER_API setzen oder in .env eintragen")


# --- Zuordnung -------------------------------------------------------------


@dataclass
class Target:
    column: str
    meter: str
    register_id: str
    register_code: str
    decimals: int
    installed_at: datetime
    # eigene Ablesungen (nicht über die API) als Kalibrierpunkte
    anchors: list[tuple[datetime, Decimal]]


def parse_time(s: str) -> datetime:
    return datetime.fromisoformat(s.replace("Z", "+00:00")).astimezone(timezone.utc)


def find_target(api: Api, meters: list[dict], name: str, direction: str, column: str) -> Target:
    meter = next((m for m in meters if m["name"] == name), None)
    if meter is None:
        sys.exit(f"Messstelle „{name}“ nicht gefunden; vorhanden: {', '.join(m['name'] for m in meters)}")
    register = next((r for r in meter["registers"] if r["direction"] == direction), None)
    if register is None:
        sys.exit(f"Messstelle „{name}“ hat kein Register mit Richtung {direction}")
    detail = api.call("GET", f"/meters/{meter['id']}")
    device = next(d for d in detail["devices"] if d["removed_at"] is None)
    anchors = [
        (parse_time(r["read_at"]), Decimal(r["value"]))
        for r in api.readings(meter["id"])
        if r["register_id"] == register["id"] and r["source"] != "api"
    ]
    return Target(
        column=column,
        meter=name,
        register_id=register["id"],
        register_code=register["code"],
        decimals=device["decimal_places"],
        installed_at=parse_time(device["installed_at"]),
        anchors=sorted(anchors),
    )


def counter_value(target: Target, energy: Energy, t: datetime) -> Decimal:
    """Zählerstand zum Zeitpunkt `t` aus SENEC-Energie und Kalibrierpunkten."""
    col, anchors = target.column, target.anchors
    before = [a for a in anchors if a[0] <= t]
    after = [a for a in anchors if a[0] > t]
    if before and after:
        (ta, va), (tb, vb) = before[-1], after[0]
        total = energy.between(col, ta, tb)
        share = energy.between(col, ta, t) / total if total > 0 else (t - ta) / (tb - ta)
        value = va + (vb - va) * Decimal(str(share))
    elif before:
        ta, va = before[-1]
        value = va + Decimal(str(energy.between(col, ta, t)))
    elif after:
        tb, vb = after[0]
        value = vb - Decimal(str(energy.between(col, t, tb)))
    else:
        value = Decimal(str(energy.at(col, t)))
    step = Decimal(1).scaleb(-target.decimals)
    return value.quantize(step, rounding=ROUND_HALF_UP)


# --- Ablauf ----------------------------------------------------------------


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("year", nargs="?", type=int, help="Jahr der Kalenderwoche")
    p.add_argument("week", nargs="?", type=int, help="Kalenderwoche (wie im Dateinamen)")
    p.add_argument("--file", help="Wochendatei statt Jahr und Woche")
    p.add_argument("--dir", default=DEFAULT_DIR, help="Verzeichnis der SENEC-CSVs")
    p.add_argument("--url", default="http://localhost:8080", help="Adresse der Zähler-Instanz")
    p.add_argument("--env-file", help="Datei mit ZAEHLER_API=...")
    p.add_argument("--grid-meter", default="Strom", help="Hausanschluss (Bezug und Einspeisung)")
    p.add_argument("--pv-meter", default="PV Erzeugung", help="Erzeugungszähler; leer = auslassen")
    p.add_argument("--battery-meter", default="", help="Speicherzähler (Ladung = Bezug, Entnahme = Einspeisung)")
    p.add_argument("--dry-run", action="store_true", help="nur anzeigen, nichts senden")
    p.add_argument("--all", action="store_true", help="alle Wochen des Verzeichnisses importieren")
    p.add_argument("--quiet", action="store_true", help="nur Zusammenfassung je Woche ausgeben")
    args = p.parse_args()

    if args.all:
        paths = sorted(
            glob.glob(os.path.join(args.dir, "*.csv")),
            key=lambda x: tuple(int(g) for g in reversed(FILE_RE.search(x).groups())),
        )
    elif args.file:
        paths = [args.file]
    elif args.year and args.week:
        paths = [week_file(args.dir, args.year, args.week)]
    else:
        p.error("Jahr und Kalenderwoche, --file oder --all angeben")

    api = Api(args.url, load_token(args.env_file))
    household = api.call("GET", "/household")
    tz = ZoneInfo(household["timezone"])
    meters = api.call("GET", "/meters")

    targets = [
        find_target(api, meters, args.grid_meter, "import", GRID_IMPORT),
        find_target(api, meters, args.grid_meter, "export", GRID_EXPORT),
    ]
    if args.pv_meter:
        targets.append(find_target(api, meters, args.pv_meter, "import", GENERATION))
    if args.battery_meter:
        targets.append(find_target(api, meters, args.battery_meter, "import", BATTERY_CHARGE))
        targets.append(find_target(api, meters, args.battery_meter, "export", BATTERY_DISCHARGE))

    # Alle Dateien laden: Kalibrierpunkte können Wochen oder Monate entfernt sein
    all_samples: list[Sample] = []
    for path in sorted(glob.glob(os.path.join(os.path.dirname(paths[0]) or ".", "*.csv"))):
        all_samples.extend(read_csv(path, tz))
    energy = Energy(all_samples)

    total: dict[str, int] = {}
    for path in paths:
        for k, n in import_week(api, path, tz, household, targets, energy, args).items():
            total[k] = total.get(k, 0) + n
    if len(paths) > 1:
        print(f"\nGesamt ({len(paths)} Wochen): " + ", ".join(f"{k} {n}" for k, n in sorted(total.items())))


def import_week(api: Api, week_path: str, tz: ZoneInfo, household: dict,
                targets: list[Target], energy: Energy, args) -> dict[str, int]:
    """Stände einer Wochendatei berechnen und senden; Zähler je Ergebnis."""
    week_samples = read_csv(week_path, tz)
    if not week_samples:
        print(f"{os.path.basename(week_path)}: keine Messwerte")
        return {}
    first_day = min(s.at for s in week_samples).astimezone(tz).date()
    last_day = max(s.at for s in week_samples).astimezone(tz).date()

    # Tagesgrenzen 00:00 vom ersten Tag bis zum Tag nach dem letzten
    days = [first_day + timedelta(days=i) for i in range((last_day - first_day).days + 2)]
    moments = [datetime.combine(d, time(0), tz).astimezone(timezone.utc) for d in days]
    moments = [m for m in moments if energy.start - MAX_GAP <= m <= energy.end + MAX_GAP]

    head = (f"Woche {os.path.basename(week_path)}: {first_day:%d.%m.%Y} – {last_day:%d.%m.%Y}, "
            f"Haushalt {household['name']} ({tz.key})")
    if not args.quiet:
        print(head)
    items, plan, skipped = [], [], 0
    for target in targets:
        values = [(m, counter_value(target, energy, m)) for m in moments]
        anchor_note = f"{len(target.anchors)} eigene Ablesungen" if target.anchors else "ohne eigene Ablesung, Zählung ab SENEC-Beginn"
        if not args.quiet:
            print(f"\n{target.meter} / {target.register_code} ← {target.column} ({anchor_note})")
        for m, v in values:
            skip = m < target.installed_at
            skipped += skip
            day_kwh = ""
            nxt = m + timedelta(days=1)
            if nxt <= energy.end:
                day_kwh = f"{energy.between(target.column, m, nxt):8.2f} kWh am Tag"
            if not args.quiet:
                print(f"  {m.astimezone(tz):%a %d.%m.%Y %H:%M}  {v:>12}  {day_kwh}"
                      f"{'  (vor dem Einbau, übersprungen)' if skip else ''}")
            if not skip:
                items.append({"register_id": target.register_id,
                              "read_at": m.isoformat().replace("+00:00", "Z"),
                              "value": str(v)})
                plan.append((target, m, v))

    counts: dict[str, int] = {"skipped": skipped} if skipped else {}
    if args.dry_run or not items:
        counts["planned"] = len(items)
    else:
        results = api.call("POST", "/readings", {"readings": items})["results"]
        for (target, m, v), r in zip(plan, results):
            counts[r["status"]] = counts.get(r["status"], 0) + 1
            if r["status"] == "error":
                print(f"  Fehler {target.meter}/{target.register_code} {m.astimezone(tz):%d.%m.%Y}: {r['error']}")
    summary = ", ".join(f"{k} {n}" for k, n in sorted(counts.items()))
    print(f"{os.path.basename(week_path)} ({first_day:%d.%m.%Y}): {summary}" if args.quiet
          else f"\nErgebnis: {summary}")
    return counts

if __name__ == "__main__":
    main()
