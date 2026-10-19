# Kubernetes

Voraussetzungen: Helm 3, für `database.type=cnpg` der
[CloudNativePG-Operator](https://cloudnative-pg.io/).

## Installation mit CNPG

```bash
kubectl create namespace zaehler
kubectl -n zaehler create secret generic zaehler-ai --from-literal=api-key=sk-or-…
kubectl -n zaehler create secret generic zaehler-oidc --from-literal=client-secret=…

helm upgrade --install zaehler deploy/helm/zaehler -n zaehler \
  --set image.repository=code.do9ita.de/tom/zaehler \
  --set publicUrl=https://zaehler.example.com \
  --set ingress.enabled=true --set ingress.hosts[0].host=zaehler.example.com \
  --set oidc.enabled=true,oidc.issuerUrl=https://auth.example.com/realms/home,oidc.existingSecret=zaehler-oidc \
  --set ai.enabled=true,ai.existingSecret=zaehler-ai
```

Das Chart legt den Cluster `<release>-db` an; die App liest die Verbindung aus
dem vom Operator erzeugten Secret `<release>-db-app` (Key `uri`).

## Varianten

| Ziel | Werte |
|---|---|
| Bestehender CNPG-Cluster | `database.type=existing`, `database.existing.secretName=<cluster>-app` |
| SQLite | `database.type=sqlite` (1 Replica, PVC, `Recreate`) |
| Mehrere Replicas | `database.type=cnpg\|existing`, `storage.kind=s3`, `replicaCount=2+` |
| Monitoring | `serviceMonitor.enabled=true` (aktiviert auch den CNPG-PodMonitor) |
| Traces | `otel.endpoint=http://otel-collector.monitoring:4318` |
| CNPG-Sicherung | `database.cnpg.backup` (wird als `spec.backup` übernommen), `database.cnpg.scheduledBackup.enabled=true`, siehe {doc}`backup` |

## Weitere Werte

| Wert | Default | Konfiguration |
|---|---|---|
| `session.ttlHours`, `session.cookieSecure` | `720`, leer (aus `publicUrl`) | `session.*` |
| `upload.maxPhotoBytes`, `maxCsvBytes`, `maxBackupBytes` | 15 MiB, 5 MiB, 1 GiB | `upload.*` |
| `imageProcessing.maxPixels`, `jpegQuality` | `2000000`, `85` | `image.*` |
| `housekeeping.intervalMinutes`, `draftTtlHours` | `60`, `24` | `housekeeping.*` |
| `otel.endpoint`, `otel.serviceName` | leer, `zaehler` | `otel.*` |
| `storage.s3.allowHttp` | `false` | `storage.allow_http` |

Ein Foto-Upload umfasst bis zu 20 Fotos zu je `upload.maxPhotoBytes`. Die
Grenze des Ingress-Controllers entsprechend setzen, bei ingress-nginx z.B.
`ingress.annotations."nginx\.ingress\.kubernetes\.io/proxy-body-size"=320m`.

## Sicherheit

Der Container läuft als UID 65532 mit read-only Root-Dateisystem, ohne
Capabilities und ohne Service-Account-Token. Geheimnisse werden ausschließlich
über bestehende Secrets referenziert (kompatibel mit External Secrets / SOPS).
