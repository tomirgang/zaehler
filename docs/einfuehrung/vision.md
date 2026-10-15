# Vision und Ziele

## Ausgangslage

Zählerstände werden manuell abgelesen und in Papierlisten oder Tabellen
festgehalten. Ohne Auswertung sind Abweichungen vom üblichen Verbrauch, die
Deckung der Kosten durch den Abschlag und die Kosten eines Fahrzeugs pro
Kilometer erst mit der Jahresabrechnung bzw. gar nicht bekannt.

## Ziele

1. **Erfassung:** Formular oder Foto, mobil bedienbar, mit AI-Vorschlag und
   Plausibilitätsprüfung.
2. **Auswertung:** Vergleich mit Vormonat, Vorjahr und gleitenden
   Durchschnitten (3/6/12 Monate); Ampel, ob der Verbrauch im erwarteten
   Bereich liegt.
3. **Prognose:** Jahresverbrauch und Kosten, Vergleich mit den Abschlägen,
   Vorschlag für die Abschlagshöhe.
4. **Fahrzeugkosten:** Kostenbuch, Wertverlust und Kosten pro km je Jahr und
   über die Haltedauer.
5. **Photovoltaik:** Eigenverbrauch, Autarkiegrad, Ertragsüberwachung,
   Einspeisevergütung und Amortisation von PV-Anlagen.
6. **Betrieb:** ein Container, PostgreSQL (CNPG) oder SQLite, Helm-Chart und
   docker compose.

## Nicht-Ziele

- Keine automatische Auslesung von Smart Metern im MVP (optional später über
  die REST-API).
- Kein Fahrtenbuch im steuerlichen Sinn, keine Trennung privat/geschäftlich.
- Keine Mehrwährungsfähigkeit (nur EUR).
- Keine steuerlichen Auswertungen (Umsatzsteuer, Einkommensteuer, EÜR), auch nicht für PV-Anlagen.
- Keine Anbindung von Wechselrichter-Schnittstellen im Kern; automatische Quellen nutzen die REST-API.

## Stakeholder

| Rolle | Interesse |
|---|---|
| Haushaltsmitglied | Stände erfassen, Verbrauch und Kosten einsehen |
| Haushalts-Owner | Zähler, Verträge und Mitglieder verwalten |
| Betreiber (Self-Hoster) | Betrieb in k8s oder compose, Sicherheitsvorgaben (REQ_NFR_003) |
| Entwickler | Nachvollziehbare Anforderungen, testbare Fachlogik |

## Meilensteine

| Meilenstein | Inhalt |
|---|---|
| **MVP** | Haushalte/OIDC, Zähler inkl. Register und Zählerwechsel, Erfassung per Formular und Foto, Verbrauch & Vergleiche, Verbrauchsprognose, CSV-Import, Betrieb (Helm, compose, beide DBs) |
| **v1** | Tarife & Kosten, Abschläge, Kostenprognose, Fahrzeugkosten, Photovoltaik, Benachrichtigungen per E-Mail, Rechnungs- und Belegerkennung, REST-API |
| **später** | Preisdeckel, dynamische Tarife, Prognose-Unsicherheit, Vollbackup, PVGIS-Ertragserwartung, interner Zinsfuß |
