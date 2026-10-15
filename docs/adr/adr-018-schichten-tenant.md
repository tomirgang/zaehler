# ADR-018: Schichtung Handler, Service, Repository mit Tenant-Typ

```{adr} Schichten im Server und Tenant als Berechtigungsnachweis
:id: ADR_018
:status: superseded
:decides: REQ_TEN_001, REQ_NFR_003, SPEC_ARCH_LAYERS, SPEC_SEC_TENANT_SCOPE

**Datum:** 2026-10-15

Ersetzt durch {need}`ADR_022`.
```

## Kontext
Jeder Zugriff auf fachliche Daten muss auf den aktiven Haushalt beschränkt
sein. Eine vergessene Filterbedingung führt zu einem Datenleck zwischen
Haushalten. Row-Level-Security steht wegen SQLite nicht zur Verfügung
({need}`ADR_004`).

## Entscheidung
- Schichten `web` → `service` → `zaehler-db::repo`; Handler greifen nicht
  direkt auf die Datenbank zu.
- `Tenant` (Nutzer, Haushalt, Rolle) mit privaten Feldern entsteht nur über
  `HouseholdRepo::authorize` nach Prüfung der Mitgliedschaft. Alle
  fachlichen Repository-Funktionen verlangen `&Tenant`.
- Funktionen ohne Haushaltsbezug (Housekeeping) liegen in einem separaten
  `SystemRepo` mit Sichtbarkeit `pub(crate)` für das Modul `housekeeping`.

## Alternativen
- *`household_id` als einfacher Parameter*: Prüfung der Mitgliedschaft ist
  nicht erzwungen.
- *Mandant je Datenbank bzw. Schema*: aufwendig im Betrieb, mit SQLite nur
  über mehrere Dateien.

## Konsequenzen
- Der Compiler erzwingt, dass jede fachliche Abfrage einen geprüften Tenant
  erhält; die Filterbedingung selbst prüfen Tenant-Isolationstests.
- Zusätzliche Strukturen zwischen Entities und Views (Services geben keine
  Entities heraus).
