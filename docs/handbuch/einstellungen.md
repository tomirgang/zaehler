# Haushalt und Einstellungen

```{figure} bilder/einstellungen.png
:alt: Seite Einstellungen
:width: 100%

Einstellungen: Haushalt, Mitglieder, Setup-Assistent, API-Tokens, Benachrichtigungen,
Sprache, weitere Haushalte, Datensicherung und Löschen.
```

## Haushalt

Name und **Zeitzone** des Haushalts stehen oben in den Einstellungen; die
Zeitzone bestimmt, wann ein Tag oder Monat beginnt. Wer mehreren Haushalten
angehört, wechselt über das Feld **Haushalt** in der Kopfzeile.
**Weiteren Haushalt anlegen** erstellt einen neuen, in dem man Eigentümer
ist.

## Mitglieder und Einladungen

**Mitglieder verwalten** zeigt alle Personen des Haushalts mit ihrer Rolle
(Lesen, Mitglied, Eigentümer, siehe {doc}`index`). Eigentümer ändern Rollen,
entfernen Mitglieder und erstellen **Einladungen**: Die App erzeugt einen
Link, der – mit eingerichtetem E-Mail-Versand – auch per E-Mail verschickt
wird. Offene Einladungen lassen sich widerrufen. **Haushalt verlassen**
beendet den eigenen Zugriff; ein Haushalt behält immer mindestens einen
Eigentümer.

```{figure} bilder/mitglieder.png
:alt: Mitglieder und Einladungen
:width: 100%

Mitglieder mit Rolle, Einladung erstellen und offene Einladungen.
```

## Setup-Assistent

Sobald Messstellen angelegt sind, starten Eigentümer den
Setup-Assistenten (siehe {doc}`erste-schritte`) über **Setup starten** in
den Einstellungen, etwa um eine PV-Anlage oder ein Fahrzeug zu ergänzen.
Ein angefangener Entwurf erscheint dort als **Setup fortsetzen**.

## Benachrichtigungen

Jede Person stellt für sich ein, ob sie per E-Mail benachrichtigt wird:

- bei **fälligen Ablesungen** (höchstens eine E-Mail je Haushalt und Tag,
  ab 8 Uhr; nur für Mitglieder und Eigentümer),
- bei **auffälligem Verbrauch** (eine E-Mail je Ablesung, deren Verbrauch
  deutlich außerhalb der Erwartung liegt).

Ohne eingerichteten E-Mail-Versand werden die Einstellungen gespeichert und
gelten, sobald der Betreiber SMTP konfiguriert.

```{figure} bilder/benachrichtigungen.png
:alt: Benachrichtigungseinstellungen
:width: 100%

E-Mail-Adresse und Art der Benachrichtigungen.
```

## Sprache

Die Oberfläche gibt es auf Deutsch und Englisch; die Auswahl gilt für das
eigene Konto.

## Daten sichern und übertragen

**Haushalt herunterladen** erstellt wahlweise

- ein **Archiv** (ZIP mit JSON und Fotos), das sich – auch in einer anderen
  Installation – mit **Als neuen Haushalt importieren** wieder einspielen
  lässt, oder
- **CSV-Dateien** je Tabelle für Tabellenkalkulationen.

Die Sicherung der ganzen Installation übernimmt der Betreiber
(siehe {doc}`../betrieb/backup`).

**Haushalt löschen** entfernt alle Messstellen und Ablesungen des Haushalts;
zur Bestätigung ist der Name des Haushalts einzugeben.

## API-Tokens

Für Skripte und Smart-Home-Systeme legt man unter **API-Tokens verwalten**
persönliche Tokens an: Name, Rolle (höchstens die eigene) und optional ein
Ablaufdatum. Das Token wird genau einmal angezeigt. Ein Token darf, was die
Person, die es erstellt hat, mit ihrer aktuellen Rolle darf, höchstens aber
die Rolle des Tokens. Tokens lassen sich jederzeit widerrufen. Die
Schnittstelle beschreibt {doc}`../betrieb/api`.

```{figure} bilder/api-tokens.png
:alt: API-Tokens verwalten
:width: 100%

API-Tokens mit Rolle, Ablauf und letzter Nutzung.
```
