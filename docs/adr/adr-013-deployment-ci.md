# ADR-013: Helm-Chart, docker compose und Forgejo Actions

```{adr} Auslieferung über Helm und compose, CI mit Forgejo Actions
:id: ADR_013
:status: accepted
:decides: REQ_OPS_006, REQ_OPS_007, REQ_OPS_001

**Datum:** 2026-10-15
```

## Kontext
Zielumgebungen sind Kubernetes (mit CNPG) und docker compose. Das Repository
liegt auf einer Forgejo-/Gitea-Instanz.

## Entscheidung
- **Helm-Chart** `deploy/helm/zaehler`: Datenbank `cnpg` (Cluster-CR wird angelegt),
  `existing` (Secret) oder `sqlite` (PVC, 1 Replica); Storage `fs` oder `s3`;
  Secrets nur per Referenz auf bestehende Secrets.
- **compose.yaml** (SQLite) + **compose.postgres.yaml** (Override).
- **Image:** distroless `cc-debian13:nonroot`, Multi-Arch per Buildx.
- **CI:** `.forgejo/workflows/ci.yaml` mit fmt, clippy, Tests (SQLite + Postgres-Service),
  Doku-Build mit `-W`, `helm lint`, Image-Push in die Forgejo-Registry.

## Alternativen
- *Kustomize*: kein Templating; optionale Komponenten (CNPG) erfordern
  je Kombination ein eigenes Overlay.

## Konsequenzen
- Runner-Label (`docker`) und Registry-Variablen sind instanzspezifisch.
