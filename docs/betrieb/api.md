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

## Weitere Endpunkte

| Methode | Pfad | Rolle |
|---|---|---|
| GET | `/api/v1/meters` | Lesen |
| GET | `/api/v1/meters/{id}` | Lesen |
| GET | `/api/v1/meters/{id}/readings?from&to&limit&cursor` | Lesen |
| POST | `/api/v1/meters/{id}/readings` | Mitglied |
| GET | `/api/v1/meters/{id}/analysis` | Lesen |
| GET | `/api/v1/meters/{id}/costs` | Lesen |
| GET | `/api/v1/contracts`, `/api/v1/contracts/{id}` | Lesen |

Fehler sind `application/problem+json` (RFC 9457) mit `status`, bei
Validierung `errors` je Feld und bei Plausibilitätswarnungen `warnings`.
Eine Ratenbegrenzung ist Aufgabe des Ingress ({need}`ADR_023`).
