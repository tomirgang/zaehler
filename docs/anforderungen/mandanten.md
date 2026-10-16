# Mandanten und Zugriff

```{req} Haushalte als Mandanten
:id: REQ_TEN_001
:status: accepted
:priority: must
:release: MVP

Das System muss alle fachlichen Daten (Zähler, Stände, Verträge, Kosten) genau
einem **Haushalt** zuordnen. Ein Nutzer kann Mitglied mehrerer Haushalte sein und
zwischen ihnen wechseln.

**Akzeptanzkriterien**
- Ein Nutzer sieht ausschließlich Daten von Haushalten, in denen er Mitglied ist.
- Jeder Datenbankzugriff auf fachliche Daten ist nach Haushalt gefiltert
  (serverseitig erzwungen, nicht nur im UI).
- Beim ersten Login wird ein persönlicher Haushalt angelegt, sofern keine
  Einladung vorliegt.
```

```{req} Anmeldung über OIDC
:id: REQ_TEN_002
:status: accepted
:priority: must
:release: MVP

Das System muss die Anmeldung über einen OpenID-Connect-Provider (z.B. Keycloak,
Authentik, Entra ID) unterstützen. Eine eigene Passwortverwaltung entfällt.

**Akzeptanzkriterien**
- Authorization Code Flow mit PKCE; Issuer, Client-ID und Secret sind konfigurierbar.
- Nutzer werden beim ersten Login angelegt (Schlüssel: Issuer + Subject).
- Sitzung über sicheres, `HttpOnly`-, `SameSite=Lax`-Cookie; Abmelden beendet die Sitzung.
- Ohne OIDC-Konfiguration startet das System nur mit explizitem Entwicklungsmodus.
```

```{req} Rollen im Haushalt
:id: REQ_TEN_003
:status: accepted
:priority: should
:release: v1

Das System soll die Rollen `owner` (verwalten), `member` (erfassen/bearbeiten)
und `viewer` (nur lesen) je Haushalt unterscheiden.

**Akzeptanzkriterien**
- Nur `owner` können Mitglieder, Zähler und Verträge anlegen oder löschen.
- `viewer` können keine Daten verändern (auch nicht über die API).
- Ein Haushalt hat immer mindestens einen `owner`.
```

```{req} Einladungen
:id: REQ_TEN_004
:status: accepted
:priority: should
:release: v1

Das System soll es `owner`n ermöglichen, weitere Personen per Einladungslink
(optional per E-Mail versendet) in den Haushalt einzuladen.

**Akzeptanzkriterien**
- Einladungslinks sind einmalig verwendbar und laufen nach 7 Tagen ab.
- Die Rolle wird bei der Einladung festgelegt.
```

```{req} Haushaltsname sichtbar und änderbar
:id: REQ_TEN_005
:status: accepted
:priority: must
:release: MVP

Das System muss den aktiven Haushalt auf jeder Seite eindeutig als Haushalt
gekennzeichnet anzeigen und `owner`n ermöglichen, den Namen des Haushalts zu
ändern.

**Akzeptanzkriterien**
- Die Titelleiste zeigt den Namen des aktiven Haushalts mit der Bezeichnung
  „Haushalt“, getrennt vom Namen des angemeldeten Nutzers.
- Von der Titelleiste führt ein direkter Weg zum Umbenennen.
- Nach dem Speichern erscheint der neue Name sofort in der Titelleiste und in
  der Haushaltsauswahl.
```
