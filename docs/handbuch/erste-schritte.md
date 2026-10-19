# Erste Schritte

## Anmelden

Zähler meldet über den Identity Provider der Installation an (z.B. Keycloak).
Nach der ersten Anmeldung legt die App automatisch einen Haushalt an, in dem
man Eigentümer ist. Wer eine Einladung erhalten hat, öffnet den
Einladungslink und tritt damit dem bestehenden Haushalt bei
(siehe {doc}`einstellungen`).

## Setup-Assistent

In einem leeren Haushalt bietet die Übersicht **Setup starten** an. Sobald
Messstellen existieren, finden Eigentümer den Assistenten unter
**Einstellungen → Setup-Assistent** (und in der Messstellenliste). Der
Assistent richtet in sieben Schritten typische Kombinationen ein:

1. Hausanschluss Strom, wahlweise mit Einspeiseregister
2. Wärmepumpe als Zwischenzähler oder mit eigenem Zähler
3. PV-Anlage mit Erzeugungs- und optional Speicherzähler
4. Elektrofahrzeug oder Plug-in-Hybrid mit optionalem Wallbox-Zähler
5. weitere Medien (Gas, Wasser, Wärme)
6. Verträge
7. Zusammenfassung

Jeder Baustein außer dem Hausanschluss ist optional: **Diesen Baustein
einrichten** blendet seine Felder ein, **Überspringen** lässt ihn aus. Felder
erscheinen nur, wenn sie zur bisherigen Auswahl passen; Pflichtfelder sind mit
`*` markiert. Für jeden Zähler kann statt eines neuen eine **vorhandene
Messstelle** gewählt werden – so lässt sich ein bestehendes Setup später
erweitern. Je Zähler kann ein Foto hochgeladen werden; mit eingerichteter
KI-Erkennung belegt es Zählernummer und Startstand vor.

```{figure} bilder/setup-assistent.png
:alt: Setup-Assistent, Schritt Elektrofahrzeug
:width: 100%

Schritt „Elektrofahrzeug“: Mit „Neuen Wallboxzähler anlegen“ erscheinen
Name und Startstand der Wallbox, mit „Anschaffung erfassen“ die Angaben für
den Wertverlust.
```

Angelegt wird erst mit **Setup abschließen** in der Zusammenfassung, alle
Objekte gemeinsam. Bis dahin bleibt der Entwurf erhalten und kann später
fortgesetzt oder mit **Entwurf verwerfen** gelöscht werden.

## Messstelle anlegen

Unter **Messstellen → Neue Messstelle** entsteht ein einzelner Zähler.

```{figure} bilder/messstelle-anlegen.png
:alt: Formular „Neue Messstelle“
:width: 100%

Name, Art und Einheit, Zählernummer, Vor- und Nachkommastellen,
Verwendungszweck, übergeordnete Messstelle, Ableseplan, Erwartung und
Register mit Startstand.
```

Wichtige Angaben:

- **Art**: Strom, Gas, Wasser, Wärme oder benutzerdefiniert mit eigener
  Einheit. Fahrzeuge entstehen unter **Fahrzeuge** (siehe {doc}`fahrzeuge`).
- **Register**: je Zählwerk ein Register mit Richtung Bezug oder Einspeisung
  und dem Startstand beim Einbau.
- **Ableseplan**: alle *n* Tage oder monatlich an einem festen Tag; daraus
  ergibt sich die Fälligkeit auf der Übersicht und die Erinnerung per E-Mail.
- **Maximaler Tagesverbrauch**: Stände, die einen höheren Verbrauch ergeben,
  muss man bei der Erfassung bestätigen.
- **Beginn des Abrechnungsjahres**: zusätzlich zum Kalenderjahr eine
  Prognose für das Abrechnungsjahr des Versorgers.
- **Vorkommastellen**: Zähler mit wenigen Stellen laufen über; die App
  erkennt den Überlauf.
- **Verwendungszweck** und **übergeordnete Messstelle** (nur Strom):
  Hausstrom, Wärmepumpe oder Wallbox; mit übergeordneter Messstelle wird der
  Zähler zum Zwischenzähler, z.B. Wärmepumpe hinter dem Hausstrom.
- **Erwartung** (Jahresziel, Toleranz, Saisonprofil): Grundlage der Ampel,
  solange noch keine zwei Vorjahreswerte desselben Monats vorliegen.
- **Einbau und Startstände am**: Zeitpunkt, ab dem der Zähler misst; Stände
  davor lehnt die App ab.

**Mit Foto anlegen** startet stattdessen mit einem Foto des Zählers; auf der
Prüfseite „Neue Messstelle aus diesem Foto“ wählen. Mit eingerichteter
KI-Erkennung sind Zählernummer, Art und Startstand vorbelegt.

## Messstellen verwalten

```{figure} bilder/messstellen.png
:alt: Liste der Messstellen
:width: 100%

Die Liste der Messstellen mit Art, Einheit und letztem Stand.
```

Auf der Seite einer Messstelle stehen oben die Aktionen:

- **Bearbeiten**: Name, Ableseplan, Register und weitere Angaben ändern.
- **Zählerwechsel**: Endstand des alten und Startstand des neuen Geräts zum
  selben Zeitpunkt; die Verbrauchsreihe läuft ohne Bruch weiter.
- **Archivieren**: Die Messstelle verschwindet aus Übersicht, Erfassung und
  Erinnerungen, ihre Auswertungen bleiben. **Wieder aktivieren** hebt das auf.
- **Löschen**: entfernt die Messstelle endgültig mit allen Ablesungen, Fotos
  und Verträgen. Solange ein Zwischenzähler, eine PV-Anlage oder ein Fahrzeug
  auf sie verweist, lehnt die App das Löschen mit einem Hinweis ab.

```{figure} bilder/zaehlerwechsel.png
:alt: Formular Zählerwechsel
:width: 100%

Zählerwechsel: End- und Startstände je Register, neue Zählernummer.
```
