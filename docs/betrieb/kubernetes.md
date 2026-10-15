# Kubernetes

Voraussetzungen: Helm 3, für `database.type=cnpg` der
[CloudNativePG-Operator](https://cloudnative-pg.io/).

## Installation mit CNPG

```bash
kubectl create namespace zaehler
kubectl -n zaehler create secret generic zaehler-ai --from-literal=api-key=sk-or-…
kubectl -n zaehler create secret generic zaehler-oidc --from-literal=client-secret=…

helm upgrade --install zaehler deploy/helm/zaehler -n zaehler \
  --set image.repository=git.example.com/owner/zaehler \
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

## Sicherheit

Der Container läuft als UID 65532 mit read-only Root-Dateisystem, ohne
Capabilities und ohne Service-Account-Token. Geheimnisse werden ausschließlich
über bestehende Secrets referenziert (kompatibel mit External Secrets / SOPS).
