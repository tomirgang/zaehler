# ADR-023: REST-API im Web-Adapter mit OpenAPI aus dem Code

```{adr} REST-API mit utoipa und persönlichen Bearer-Tokens
:id: ADR_023
:status: proposed
:decides: REQ_API_001, REQ_API_002, REQ_API_003, SPEC_API_AUTH, SPEC_API_REST

**Datum:** 2026-10-16
```

## Kontext
Die REST-API ({need}`REQ_API_001`) soll dieselben Anwendungsfälle wie die
Oberfläche verwenden und eine OpenAPI-Beschreibung aus dem Code erzeugen.
Clients wie Home Assistant senden Zählerstände ohne Browser, also ohne
Sitzung und ohne CSRF-Token. Tokens müssen widerrufbar und an Haushalt und
Rolle gebunden sein ({need}`REQ_API_002`).

## Entscheidung
- Die API ist ein Modul `api` in `zaehler-web` (Driving-Adapter gemäß
  {need}`ADR_022`) mit eigenem Router unter `/api/v1`; sie ruft dieselben
  Anwendungsfälle auf wie die Seiten.
- OpenAPI 3.1 über `utoipa` (Makros an Handlern und DTOs); ausgeliefert unter
  `/api/v1/openapi.json`. DTOs der API sind eigene Typen im Web-Adapter, nicht
  die Kerntypen.
- Authentifizierung nur über `Authorization: Bearer` mit persönlichen Tokens
  (zufällig, nur als SHA-256 gespeichert). Sitzungs-Cookies werden unter
  `/api/v1` ignoriert; dadurch ist kein CSRF-Schutz nötig.
- Dezimalwerte werden als JSON-Strings übertragen (wie {need}`SPEC_AI_EXTRACTION`).

## Alternativen
- *Eigenes Crate `zaehler-api`*: trennt den Code, verdoppelt aber Router,
  Fehlerabbildung, Extraktoren und Konfiguration; kein Vorteil für die
  Abhängigkeitsregeln.
- *OAuth2 Client Credentials über den OIDC-Provider*: kein zusätzlicher
  Token-Speicher, aber Einrichtung je Client im Provider; für Home Assistant
  und Skripte zu aufwendig.
- *Handgeschriebene OpenAPI-Datei*: weicht ohne Prüfung vom Code ab.

## Konsequenzen
- Neue Abhängigkeiten `utoipa` 5 und `utoipa-axum` in `zaehler-web`.
- Tokens mit SHA-256 ohne Salt sind ausreichend, da sie 256 Bit Zufall
  enthalten; ein langsamer Passwort-Hash ist nicht nötig.
- Ratenbegrenzung ist nicht Teil der Anwendung; sie bleibt Aufgabe des
  Ingress.
