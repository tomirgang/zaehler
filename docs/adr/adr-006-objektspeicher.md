# ADR-006: Objektspeicher über `object_store` (Dateisystem oder S3)

```{adr} Objektspeicher Dateisystem oder S3
:id: ADR_006
:status: accepted
:decides: REQ_OPS_010, REQ_RDG_004, REQ_OPS_008

**Datum:** 2026-10-15
```

## Kontext
Fotos und Rechnungs-PDFs sollen nicht in der Datenbank liegen (Backup-Größe,
Performance). In k8s mit mehreren Replicas braucht es geteilten Speicher.

## Entscheidung
Crate **`object_store`** mit Backends `LocalFileSystem` (compose, Einzelinstanz)
und `AmazonS3` (S3-kompatibel: MinIO, Garage, Ceph RGW, AWS). In der DB steht
nur der Objektschlüssel `households/{id}/readings/{uuid}.jpg`.

## Alternativen
- *BLOBs in der DB*: keine zusätzliche Komponente; DB-Backups wachsen mit jedem Foto.
- *RWX-Volumes*: in vielen Clustern nicht verfügbar.

## Konsequenzen
- Mehrere Replicas nur mit S3 (Helm-Chart validiert das).
- Löschen eines Haushalts muss Objekte mit entfernen.
