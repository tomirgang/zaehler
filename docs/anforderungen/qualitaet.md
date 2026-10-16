# Qualitätsanforderungen

```{req} Mobile-first und PWA
:id: REQ_NFR_001
:status: accepted
:priority: must
:release: MVP

Das System muss auf Smartphones vollständig bedienbar sein und als Progressive
Web App installierbar sein (Manifest, Icons, Kamerazugriff über
`<input type="file" capture>`).
```

```{req} Mehrsprachigkeit
:id: REQ_NFR_002
:status: accepted
:priority: must
:release: MVP

Das System muss eine deutsch- und englischsprachige Oberfläche bieten
(Standard: Deutsch, Auswahl je Nutzer, sonst `Accept-Language`). Zahlen- und
Datumsformate folgen der gewählten Sprache.
```

```{req} Sicherheit
:id: REQ_NFR_003
:status: accepted
:priority: must
:release: MVP

Das System muss gängige Web-Risiken (OWASP Top 10) adressieren: Schutz vor
SQL-Injection (ausschließlich parametrisierte Abfragen, technisch erzwungen),
CSRF-Schutz für zustandsändernde Requests, sichere Cookies,
Content-Security-Policy ohne `unsafe-inline`-Skripte, Validierung von Uploads
(Typ, Größe), keine Geheimnisse in Logs.
```

```{req} Datenschutz
:id: REQ_NFR_004
:status: accepted
:priority: must
:release: MVP

Das System muss vollständig selbst betreibbar sein; externe Dienste (OpenRouter,
SMTP) sind optional. Ein Haushalt kann vollständig gelöscht werden
(inkl. Fotos).
```

```{req} Rechengenauigkeit
:id: REQ_NFR_005
:status: accepted
:priority: must
:release: MVP

Das System muss Mengen und Geldbeträge mit exakter Dezimalarithmetik
verarbeiten und speichern (keine Gleitkommazahlen). Rundung erfolgt erst bei
der Anzeige (Geld kaufmännisch auf Cent).
```

```{req} Antwortzeiten
:id: REQ_NFR_006
:status: accepted
:priority: should
:release: MVP

Seiten sollen bei 20 Zählern und 10 Jahren Daten serverseitig in p95 < 300 ms
gerendert werden (ohne AI-Aufrufe).
```

```{req} Barrierefreiheit
:id: REQ_NFR_007
:status: accepted
:priority: should
:release: v1

Die Oberfläche soll sich an WCAG 2.2 AA orientieren (Kontraste, Tastaturbedienung,
semantisches HTML, Diagramme mit Tabellenalternative).
```

```{req} Requirements-Tracing
:id: REQ_NFR_008
:status: accepted
:priority: must
:release: MVP

Anforderungen, Spezifikationen, Code und Tests müssen durchgängig verknüpft
sein. Der Dokumentations-Build in der CI muss bei toten Trace-Links fehlschlagen.
```

```{req} Rückmeldung bei laufenden Aktionen
:id: REQ_NFR_009
:status: accepted
:priority: should
:release: MVP

Die Oberfläche soll nach dem Auslösen einer Aktion (Button, Formular) sofort
anzeigen, dass der Request läuft: Ladeindikator (z.B. Spinner) am auslösenden
Element, das bis zur Antwort deaktiviert bleibt, damit keine doppelten Requests
entstehen. Das gilt insbesondere für lang laufende Requests wie die
Foto-Auswertung ({need}`REQ_AI_001`). Der Ladezustand muss für Hilfstechnologien
erkennbar sein (`aria-busy`, {need}`REQ_NFR_007`) und ohne Inline-Skripte
auskommen ({need}`REQ_NFR_003`).
```
