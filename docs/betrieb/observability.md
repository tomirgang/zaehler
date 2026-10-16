# Logs, Metriken und Traces

Siehe {need}`REQ_OPS_004`, {need}`SPEC_OPS_METRICS` und {need}`SPEC_OPS_OTEL`.

## Logs

`log.format = json` (Standard) schreibt eine JSON-Zeile je Ereignis auf
stdout. Ereignisse innerhalb eines HTTP-Requests tragen die Felder des
Request-Spans `http_request`:

| Feld | Inhalt |
|---|---|
| `request_id` | Wert des Headers `x-request-id`; ohne Header eine neue UUID. Die Antwort enthält denselben Header. |
| `http.route` | Route-Template, z.B. `/meters/{id}` (keine IDs) |
| `http.request.method`, `http.response.status_code` | Methode und Status |
| `household`, `user` | ID des aktiven Haushalts und des angemeldeten Nutzers |

Zählerwerte, Fotos, Tokens und E-Mail-Adressen werden nicht geloggt. Die
Referenz auf Fehlerseiten (`Referenz: …`) steht im Feld `reference` des
zugehörigen Log-Eintrags.

## Metriken

Prometheus-Format unter `GET /metrics` auf `server.metrics_listen`
(Standard Port 9090, nicht über den Ingress erreichbar).

| Metrik | Labels |
|---|---|
| `http_requests_total`, `http_request_duration_seconds` | `method`, `path`, `status` |
| `zaehler_readings_created_total` | `source` |
| `zaehler_ai_requests_total` | `outcome` (`ok`, `error`, `timeout`, `invalid`) |
| `zaehler_ai_request_duration_seconds` | |
| `zaehler_ai_corrections_total` | `corrected` |
| `zaehler_import_rows_total` | `result` |
| `zaehler_housekeeping_removed_total` | `kind` (`draft`, `tmp`, `session`) |

## Traces

Mit `otel.endpoint` (Basis-URL eines OTLP/HTTP-Empfängers, z.B.
`http://otel-collector:4318`) oder `OTEL_EXPORTER_OTLP_ENDPOINT` exportiert
der Server Traces per OTLP/HTTP (Protobuf); ohne Endpunkt ist der Export
abgeschaltet. Weitere Variablen des OTLP-Exporters
(`OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_EXPORTER_OTLP_TIMEOUT`) werden
berücksichtigt. Ein eingehender Header `traceparent` (W3C Trace Context) wird
als Eltern-Span übernommen.

| Span | Attribute |
|---|---|
| `http_request` (Name `GET /meters/{id}`) | Route, Methode, Status, Request-ID, Haushalt, Nutzer |
| Anwendungsfall, z.B. `RecordReadings`, `MeterAnalysis` | |
| `db.transaction` | Dauer der Transaktion bis `commit` oder Abbruch |
| `blob.put`, `blob.get`, `blob.copy`, `blob.delete`, … | |
| `ai.recognize` | `model`, `outcome`; ohne Bildinhalt |

Lokal: `docker compose -f compose.yaml -f compose.otel.yaml up -d` startet
einen Collector, der Traces und Metriken in seinem Log ausgibt.
