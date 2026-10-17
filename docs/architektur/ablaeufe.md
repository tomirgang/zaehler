# Abläufe

## Foto-Erfassung

```{spec} Ablauf Foto → Vorschlag → Bestätigung
:id: SPEC_AI_FLOW
:status: implemented
:satisfies: REQ_AI_001, REQ_AI_002, REQ_AI_003, REQ_RDG_004

1. Upload per HTMX-Formular (`multipart`, Grenze `upload.max_photo_bytes`, JPEG/PNG/WebP;
   {need}`SPEC_SEC_UPLOADS`).
2. Server normalisiert das Bild ({need}`SPEC_IMG_PROCESSING`) und legt nur das
   normalisierte JPEG im Objektspeicher ab.
3. `zaehler-ai` ruft OpenRouter mit Structured Output auf ({need}`SPEC_AI_EXTRACTION`).
4. Zuordnung: Zählernummer → Gerät → Messstelle; sonst Art + Wertebereich; sonst Auswahl.
5. Plausibilitätsprüfung ({need}`SPEC_RDG_PLAUSIBILITY`), Anzeige des Vorschlags
   mit Foto-Ausschnitt; Nutzer bestätigt oder korrigiert.
6. Speichern der Ablesung (`source = photo`, `ai_confidence`, `photo_key`) und
   des zugehörigen Entwurfs in `ai_drafts` (Vorschlag und bestätigter Wert).
```

```{mermaid}
sequenceDiagram
  actor U as Nutzer
  participant S as zaehler-server
  participant O as Objektspeicher
  participant A as OpenRouter
  U->>S: POST /capture (Bild)
  S->>O: normalisiertes JPEG speichern
  S->>A: chat/completions (Bild, Schema)
  A-->>S: JSON-Vorschlag
  S->>S: Zuordnung + Plausibilität
  S-->>U: 303 zum Bestätigungsformular
  U->>S: bestätigen oder korrigieren
  S->>S: Ablesung und Entwurf (ai_drafts) speichern
```

```{spec} Strukturierte Extraktion über OpenRouter
:id: SPEC_AI_EXTRACTION
:status: implemented
:satisfies: REQ_AI_001, REQ_AI_004, REQ_AI_005, REQ_AI_006

OpenAI-kompatibler Endpunkt `POST {base_url}/chat/completions` mit
`response_format = json_schema (strict)`, `temperature = 0`. Bilder als
Data-URL (`image_url`), PDFs als `file`-Part. Dezimalwerte werden als String
übertragen. Antworten, die nicht dem Schema entsprechen, sind Fehler (keine
Heuristik). Modell, Base-URL und Timeout sind konfigurierbar.
```

## Anmeldung

```{spec} OIDC-Login und Sitzung
:id: SPEC_TEN_OIDC
:status: implemented
:satisfies: REQ_TEN_002, REQ_TEN_001

Discovery über `{issuer}/.well-known/openid-configuration`; Code Flow + PKCE
(`openidconnect`-Crate). Nach Callback: Nutzer per (Issuer, Subject) laden oder
anlegen, Sitzung in der DB (`tower-sessions`), Cookie `HttpOnly; Secure;
SameSite=Lax`. Der aktive Haushalt ist Teil der Sitzung und wird bei jedem
Request gegen die Mitgliedschaften geprüft.
```

## Hintergrundjobs

```{spec} Scheduler mit Leader-Lock
:id: SPEC_JOB_SCHEDULER
:status: implemented
:satisfies: REQ_NTF_001, REQ_NTF_004, REQ_OPS_008

Ein In-Process-Scheduler (Tokio-Intervall, 1 min) führt Jobs aus: Fälligkeiten
prüfen, Erinnerungen und Alarme versenden. Bei PostgreSQL hält genau eine
Replica den Leader-Lock (`pg_try_advisory_lock`); bei SQLite gibt es nur eine
Instanz. `job_runs` protokolliert den letzten Lauf je Job; Doppelversand nach
Neustarts verhindert das Versandprotokoll `notification_log`
({need}`SPEC_NTF_REMINDERS`, {need}`SPEC_NTF_ALARMS`). Takt
`scheduler.tick_seconds` (60 s); `DispatchOutbox` läuft in jedem Takt,
`SendReminders` stündlich, Housekeeping im bisherigen Intervall.
```

## Import

```{spec} CSV-Import
:id: SPEC_IMP_CSV
:status: implemented
:satisfies: REQ_IMP_001

Zweistufig: (1) Upload + Spaltenzuordnung + Vorschau (nicht persistiert, Ergebnis
der Plausibilitätsprüfung je Zeile), (2) Übernahme in einer Transaktion.
Duplikate = gleiches Register, gleicher Zeitpunkt, gleicher Wert.
```

## Photovoltaik

```{spec} Sammelerfassung je Hausanschluss
:id: SPEC_PV_BULK_CAPTURE
:status: implemented
:satisfies: REQ_PV_003
:links: SPEC_RDG_FORM, SPEC_AI_DRAFTS

Formular: alle Register der dem Hausanschluss zugeordneten Messstellen
(`grid`, `generation`, `battery`) mit einem gemeinsamen Zeitpunkt; Prüfung und
Speichern wie {need}`SPEC_RDG_FORM`, alle Ablesungen in einer Transaktion.
Fotos: mehrere Dateien in einem Upload; jedes Foto wird einzeln normalisiert,
erkannt und zugeordnet ({need}`SPEC_AI_DRAFTS`), die Ergebnisse erscheinen in
einem gemeinsamen Bestätigungsformular mit einem Zeitpunkt. Fotos ohne
eindeutige Zuordnung erhalten eine Auswahl der Messstellen des Hausanschlusses.

Umsetzung: Formular unter `/pv/grid/{meter}/readings`
(`RecordBulkReadings`); Warnungen erfordern eine Bestätigung, gespeichert
wird in einer Transaktion mit `readings_recorded` je Messstelle. Fotos
laufen über die Mehrfach-Erfassung (`/capture` mit Feld `grid`, Analyse im
Hintergrund) und erscheinen als offene Entwürfe im selben Formular; erkannte
Messstelle, Register, Wert und Aufnahmezeit werden vorbelegt, die Zuordnung
ist je Foto wählbar („nicht verwenden“ möglich). Beim Speichern erhält jede
Ablesung der zugeordneten Messstelle eine Kopie des Fotos
(`source = photo`), der Entwurf wird abgeschlossen.
```

```{spec} PV-Darstellung
:id: SPEC_PV_CHARTS
:status: implemented
:satisfies: REQ_PV_012
:links: SPEC_ANA_CHARTS

Seite je Hausanschluss mit PV: (1) gestapelte Monatsbalken Erzeugung
(Eigenverbrauch, Einspeisung), (2) gestapelte Monatsbalken Gesamtverbrauch
(Eigenverbrauch, Bezug), (3) Erzeugung je Anlage gegen Erwartung mit Band,
(4) kumulierte Bilanz je Anlage mit markiertem Amortisationszeitpunkt.
Dashboard-Karte je Hausanschluss: Erzeugung des letzten vollständigen Monats,
Autarkiegrad, Eigenverbrauchsquote, Ampel der Ertragsüberwachung.
Tabellenalternative und Umsetzung gemäß {need}`SPEC_ANA_CHARTS`.
```

## Automatisch gelieferte Zählerstände

```{spec} Tägliche Verdichtung von API-Zählerständen
:id: SPEC_API_DAILY_READINGS
:status: implemented
:satisfies: REQ_API_003

Zählerstände über die REST-API werden mit `source = api` und der ID des
API-Tokens gespeichert. Je Register, Kalendertag (Zeitzone des Haushalts) und
Token bleibt höchstens eine Ablesung erhalten: eine neue Ablesung desselben
Tages ersetzt die vorhandene, wenn ihr Zeitpunkt später ist, sonst wird sie
verworfen. Manuelle Ablesungen und Importe sind davon nicht betroffen.
Plausibilitätswarnungen führen bei API-Ablesungen nicht zur Ablehnung,
sondern zu einem Eintrag im Log und einer Metrik.

Umsetzung: `IngestReadings` mit `ReadingRepository::upsert_daily` (Spalte
`readings.api_token_id`, Index über Token, Register und Zeitpunkt). Ein
ersetzter Stand behält seine ID. Ein Stand mit dem Zeitpunkt einer anderen
Ablesung desselben Geräts ergibt bei gleichem Wert `ignored`, sonst einen
Fehler je Eintrag. Metriken `zaehler_api_readings_total{result}` und
`zaehler_api_reading_warnings_total{kind}`; Beispiel für Home Assistant in
{doc}`../betrieb/api`.
