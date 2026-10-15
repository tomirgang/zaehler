# ADR-016: Sitzungen in der Datenbank, CSRF mit Synchronizer-Token

```{adr} Sitzungen in der Datenbank und CSRF-Token je Sitzung
:id: ADR_016
:status: proposed
:decides: REQ_TEN_002, REQ_NFR_003, SPEC_SEC_SESSION_CSRF

**Datum:** 2026-10-15
```

## Kontext
Nach dem OIDC-Login braucht die App eine eigene Sitzung. Der Server soll
zustandslos bleiben ({need}`REQ_OPS_008`) und mit SQLite wie mit PostgreSQL
laufen. HTMX sendet zustandsändernde Requests per `fetch`.

## Entscheidung
- `tower-sessions` mit eigenem `SessionStore` auf der Tabelle `sessions`
  (SeaORM, beide Backends). Sitzungs-ID im Cookie, Inhalt in der DB.
- CSRF-Schutz über ein Token je Sitzung (Synchronizer-Token), übertragen per
  Header `X-CSRF-Token` (HTMX) oder Formularfeld `_csrf`; zusätzlich
  Prüfung des `Origin`-Headers.

## Alternativen
- *Signierte Cookie-Sitzung ohne Speicher*: keine serverseitige Invalidierung,
  Größe des Cookies wächst mit dem Inhalt.
- *`tower-sessions-sqlx-store`*: getrennte Stores je Backend, eigenes Schema
  außerhalb der SeaORM-Migrationen.
- *Nur `SameSite=Lax`*: schützt nicht bei Requests derselben Site
  (Subdomains) und ältere Browser.

## Konsequenzen
- Ein DB-Zugriff je Request zum Laden der Sitzung.
- Abgelaufene Sitzungen entfernt das Housekeeping ({need}`SPEC_OPS_HOUSEKEEPING`).
