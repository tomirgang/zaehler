# REST-API und automatische Zählerstände

Die REST-API liegt unter `/api/v1` ({need}`SPEC_API_REST`); die
OpenAPI-Beschreibung ist ohne Anmeldung unter `/api/v1/openapi.json`
abrufbar. Jede andere Anfrage braucht ein persönliches Token im Header
`Authorization: Bearer zt_…` ({need}`SPEC_API_AUTH`). Sitzungscookies werden
unter `/api/v1` nicht ausgewertet.

## Tokens

Tokens entstehen unter *Einstellungen → API-Tokens*. Das Token wird genau
einmal angezeigt; gespeichert sind nur sein SHA-256 und die ersten acht
Zeichen. Die Rolle des Tokens ist höchstens die eigene Rolle; sinkt die
Mitgliedsrolle, gilt die niedrigere. Ohne Mitgliedschaft, nach Ablauf oder
Widerruf antwortet die API mit 401. Logs enthalten die Token-ID
(`api_token` im Request-Span), nie das Token.

Für Home Assistant genügt die Rolle *Mitglied* (Schreiben von Ständen), für
reines Lesen *Lesen*.

## Automatisch gelieferte Stände

`POST /api/v1/readings` nimmt bis zu 1000 Stände je Anfrage an (Body
höchstens 1 MiB). Ein Stand nennt das Register über `register_id` oder über
Zählernummer (`serial`) und Registercode (`register`):

```json
{
  "readings": [
    { "serial": "1EMH0012345678", "register": "1.8.0",
      "read_at": "2026-10-17T21:55:00Z", "value": "26415.3" }
  ]
}
```

Je Register, Kalendertag (Zeitzone des Haushalts) und Token bleibt eine
Ablesung: die späteste des Tages ({need}`SPEC_API_DAILY_READINGS`). Frühere
Stände desselben Tages werden verworfen (`ignored`), spätere ersetzen den
vorhandenen (`replaced`). Manuelle Ablesungen und Importe bleiben unberührt.
Unplausible Stände (fallend, zu hohe Tagesrate) werden gespeichert und als
Warnung im Log sowie in `zaehler_api_reading_warnings_total{kind}`
gezählt; `zaehler_api_readings_total{result}` zählt die Ergebnisse. Das
Ergebnis je Eintrag steht in `results` in der Reihenfolge der Anfrage, mit
`status` (`stored`, `replaced`, `ignored`, `error`) und bei Fehlern einem
Code wie `api.serial_unknown` oder `reading.in_future`.

## Beispiel: Home Assistant

`rest_command` in `configuration.yaml`, das Token liegt in `secrets.yaml`:

```yaml
rest_command:
  zaehler_strom:
    url: "https://zaehler.example.org/api/v1/readings"
    method: post
    headers:
      Authorization: !secret zaehler_token
    content_type: "application/json"
    payload: >-
      {"readings": [
        {"serial": "1EMH0012345678", "register": "1.8.0",
         "read_at": "{{ utcnow().isoformat() }}",
         "value": "{{ states('sensor.stromzaehler_bezug') }}"},
        {"serial": "1EMH0012345678", "register": "2.8.0",
         "read_at": "{{ utcnow().isoformat() }}",
         "value": "{{ states('sensor.stromzaehler_einspeisung') }}"}
      ]}
```

`secrets.yaml`:

```yaml
zaehler_token: "Bearer zt_…"
```

Automatisierung, die kurz vor Mitternacht und zusätzlich stündlich sendet;
gespeichert wird je Tag nur der späteste Stand:

```yaml
automation:
  - alias: "Zählerstände an Zähler senden"
    trigger:
      - platform: time_pattern
        minutes: 55
    condition:
      - condition: template
        value_template: >-
          {{ states('sensor.stromzaehler_bezug') not in ['unknown', 'unavailable'] }}
    action:
      - service: rest_command.zaehler_strom
```

Die Werte müssen zur Zahl der Nachkommastellen des Geräts passen; mehr
Stellen ergeben `reading.too_many_decimals`. Bei Bedarf rundet ein Filter wie
`{{ states('sensor.stromzaehler_bezug') | float | round(1) }}`.

## Beispiel: SENEC-Wochenexporte

`scripts/senec_import.py` importiert eine SENEC-Wochen-CSV (Leistung in kW,
etwa alle 5 Minuten) als tägliche Stände um 00:00 Uhr: Netzbezug und
Netzeinspeisung in den Hausanschluss, Stromerzeugung in den
Erzeugungszähler, optional Akkuladung und -entnahme in einen Speicherzähler.
Die Leistung wird mit der Trapezregel integriert (Lücken über 30 Minuten
werden nicht überbrückt); zwischen eigenen Ablesungen werden die Stände so
skaliert, dass sie diese genau treffen.

```sh
ZAEHLER_API=zt_... python3 scripts/senec_import.py 2026 41 --dry-run
python3 scripts/senec_import.py 2026 41 --grid-meter Strom --pv-meter "PV Erzeugung"
```

Stände vor dem Einbau des Geräts lehnt die API ab; für ältere Wochen muss das
Gerät entsprechend früher eingebaut sein.

## Weitere Endpunkte

Über die API sind alle Daten des Haushalts les- und schreibbar, die auch die
Oberfläche bearbeitet: Messstellen, Ablesungen, Verträge, Kosten, Fahrzeuge,
Kostenbücher, PV-Anlagen, Haushalt und Dokumente. Jeder Endpunkt ruft
denselben Anwendungsfall wie die Oberfläche auf; ein Token darf daher genau
das, was sein Ersteller in der Oberfläche darf, höchstens aber seine
Token-Rolle. Nicht erlaubte Schreibzugriffe ergeben 403, Objekte anderer
Haushalte 404. Rollen: L = Lesen, M = Mitglied, E = Eigentümer.

| Methode | Pfad | Rolle |
|---|---|---|
| GET, PUT | `/api/v1/household` | L, E |
| GET, POST | `/api/v1/meters` | L, E |
| GET, PUT, DELETE | `/api/v1/meters/{id}` | L, E, E |
| POST | `/api/v1/meters/{id}/archive`, `/restore`, `/swap` | E |
| GET, POST | `/api/v1/meters/{id}/readings?from&to&limit&cursor` | L, M |
| GET | `/api/v1/meters/{id}/history`, `/analysis`, `/costs` | L |
| GET, PUT, DELETE | `/api/v1/readings/{id}` | L, M, M |
| GET | `/api/v1/readings/{id}/photo` | L |
| GET, POST | `/api/v1/meters/{id}/contracts` | L, E |
| GET | `/api/v1/meters/{id}/contract-templates` | L |
| GET | `/api/v1/contracts` | L |
| GET, PUT, DELETE | `/api/v1/contracts/{id}` | L, E, E |
| POST | `/api/v1/contracts/{id}/positions/{position}/price-components`, `/installments` | E |
| POST | `/api/v1/contracts/{id}/adjustments`, `/settlements` | M |
| DELETE | `/api/v1/contracts/{id}/{installments\|adjustments\|settlements}/{entry}` | E bzw. M |
| GET | `/api/v1/costs?year` (mit `vehicles`: Fahrzeugkosten des Jahres) | L |
| GET, POST | `/api/v1/vehicles` | L, E |
| GET, PUT | `/api/v1/vehicles/{id}` | L, E |
| POST, DELETE | `/api/v1/vehicles/{id}/profiles`, `/profiles/{profile}` | E |
| PUT, DELETE | `/api/v1/vehicles/{id}/acquisition` | E |
| GET, POST | `/api/v1/vehicles/{id}/charging` | L, M |
| DELETE | `/api/v1/vehicles/{id}/charging/{session}` | M |
| GET | `/api/v1/vehicles/{id}/report` | L |
| GET, POST | `/api/v1/vehicles/{id}/costs?year`, `/api/v1/pv-systems/{id}/costs?year` | L, M |
| GET, PUT, DELETE | `/api/v1/asset-costs/{id}` | L, M, M |
| GET | `/api/v1/pv` | L |
| GET, POST | `/api/v1/pv-systems` | L, E |
| GET, PUT, DELETE | `/api/v1/pv-systems/{id}` | L, E, E |
| GET | `/api/v1/pv-systems/{id}/report` | L |
| GET | `/api/v1/pv-grid/{id}/analysis`, `/meters` | L |
| POST | `/api/v1/pv-grid/{id}/readings` | M |
| GET | `/api/v1/documents` | M |
| GET | `/api/v1/documents/{id}`, `/file` | L |
| DELETE | `/api/v1/documents/{id}` | M |

Schreibende Endpunkte erwarten die vollständige Ressource (`PUT` ersetzt);
die genauen Schemata stehen in `/api/v1/openapi.json`. Anlegen antwortet
mit 201 und der angelegten Ressource (Einträge eines Vertrags: mit dem
ganzen Vertrag), Löschen sowie Archivieren und Reaktivieren mit 204.
Stände über die API erhalten die Quelle `api`. Nicht über die API verfügbar
sind Anmeldung, Mitglieder und Einladungen, Benachrichtigungen, der
Setup-Assistent, Foto- und Dokumenten-Uploads sowie Import und Export.

Fehler sind `application/problem+json` (RFC 9457) mit `status`, bei
Validierung `errors` je Feld und bei Plausibilitätswarnungen `warnings`.
Eine Ratenbegrenzung ist Aufgabe des Ingress ({need}`ADR_023`).
