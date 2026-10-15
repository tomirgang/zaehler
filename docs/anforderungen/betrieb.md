# Betrieb

```{req} Container-Image
:id: REQ_OPS_001
:status: accepted
:priority: must
:release: MVP

Das System muss als ein einzelnes Binary (inkl. eingebetteter Templates und Assets) in einem
minimalen Container-Image (distroless, nicht-root, read-only Root-FS) für
`linux/amd64` und `linux/arm64` ausgeliefert werden.
```

```{req} Datenbanken PostgreSQL/CNPG und SQLite
:id: REQ_OPS_002
:status: accepted
:priority: must
:release: MVP

Das System muss wahlweise PostgreSQL (in k8s über CloudNativePG) oder SQLite
als Datenbank nutzen. Die Auswahl erfolgt allein über die Datenbank-URL;
fachliches Verhalten ist identisch.
```

```{req} Konfiguration über Datei und Umgebung
:id: REQ_OPS_003
:status: accepted
:priority: must
:release: MVP

Das System muss über eine optionale TOML-Datei und Umgebungsvariablen
(`ZAEHLER_*`, Vorrang vor Datei) konfigurierbar sein. Geheimnisse werden nie
geloggt oder ausgegeben.
```

```{req} Observability
:id: REQ_OPS_004
:status: accepted
:priority: must
:release: MVP

Das System muss strukturierte JSON-Logs schreiben, Prometheus-Metriken auf
einem separaten Port bereitstellen und optional Traces per OpenTelemetry
(OTLP) exportieren.
```

```{req} Health-Probes
:id: REQ_OPS_005
:status: accepted
:priority: must
:release: MVP

Das System muss Endpunkte für Liveness (`/healthz`) und Readiness (`/readyz`,
inkl. Datenbankprüfung) bereitstellen.
```

```{req} Helm-Chart
:id: REQ_OPS_006
:status: accepted
:priority: must
:release: MVP

Das System muss ein Helm-Chart bereitstellen, das optional einen CNPG-Cluster
anlegt oder eine bestehende Datenbank nutzt, Ingress und ServiceMonitor
unterstützt und Geheimnisse aus bestehenden Secrets bezieht.
```

```{req} docker compose
:id: REQ_OPS_007
:status: accepted
:priority: must
:release: MVP

Das System muss ein docker-compose-Setup für SQLite (Standard) und PostgreSQL
(Override) bereitstellen.
```

```{req} Horizontale Skalierung
:id: REQ_OPS_008
:status: accepted
:priority: should
:release: v1

Das System soll mit PostgreSQL und S3-Objektspeicher mit mehreren Replicas
betrieben werden können. Hintergrundjobs laufen dabei genau einmal
(Leader-Lock), Migrationen werden serialisiert.
```

```{req} Automatische Migrationen
:id: REQ_OPS_009
:status: accepted
:priority: must
:release: MVP

Das System muss Schemamigrationen beim Start (abschaltbar) oder per
Befehl `zaehler migrate` ausführen; Migrationen sind idempotent.
```

```{req} Objektspeicher
:id: REQ_OPS_010
:status: accepted
:priority: must
:release: MVP

Das System muss Fotos und Dokumente wahlweise im lokalen Dateisystem (Volume)
oder in einem S3-kompatiblen Objektspeicher ablegen.
```

```{req} Datensicherung
:id: REQ_OPS_011
:status: accepted
:priority: should
:release: v1

Die Dokumentation soll Backup und Restore für beide Datenbanken beschreiben
(CNPG: Barman Cloud / Objektspeicher; SQLite: Litestream oder `VACUUM INTO`)
und das Chart soll CNPG-Backups konfigurierbar machen.
```
