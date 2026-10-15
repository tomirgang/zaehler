# ADR-004: Haushalte als Mandanten, Anmeldung über OIDC

```{adr} Haushalte als Mandanten mit OIDC-Anmeldung
:id: ADR_004
:status: accepted
:decides: REQ_TEN_001, REQ_TEN_002, REQ_TEN_003, SPEC_TEN_OIDC

**Datum:** 2026-10-15
```

## Kontext
Mehrere Personen eines Haushalts teilen Zähler; eine Instanz kann mehrere
Haushalte (Familie, Ferienwohnung) bedienen. Self-Hoster betreiben meist schon
einen Identity Provider.

## Entscheidung
- Mandant = **Haushalt**; Nutzer ↔ Haushalt über Mitgliedschaft mit Rolle.
- Anmeldung ausschließlich über **OIDC** (Code Flow + PKCE), keine Passwörter.
- Mandantentrennung im Repository-Layer (Pflichtparameter `household_id`).

## Alternativen
- *Auth über Reverse-Proxy-Header* (oauth2-proxy): keine OIDC-Implementierung
  in der App; Header-Spoofing bei Fehlkonfiguration möglich; keine Rollen.
- *Lokale Accounts*: Passwort-Reset, MFA usw. selbst bauen.

## Konsequenzen
- Für lokale Entwicklung wird ein Dev-Login (nur mit expliziter Option) benötigt.
- Row-Level-Security in PostgreSQL wäre zusätzlich möglich, entfällt wegen SQLite.
