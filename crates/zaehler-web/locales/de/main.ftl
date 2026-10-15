# Allgemein
app-name = Zähler
nav-overview = Übersicht
nav-meters = Messstellen
nav-settings = Einstellungen
nav-logout = Abmelden
household-switch = Wechseln
action-save = Speichern
action-cancel = Abbrechen
action-edit = Bearbeiten
action-delete = Löschen
action-create = Anlegen

# Übersicht
dashboard-title = Übersicht
dashboard-empty = Noch keine Messstellen angelegt.
dashboard-create-meter = Messstelle anlegen
dashboard-last-reading = Letzter Stand
dashboard-no-reading = noch keiner
dashboard-due = Ablesung fällig am { $date }
dashboard-overdue = Ablesung überfällig seit { $date }
dashboard-record = Stand erfassen

# Messstellen
meters-title = Messstellen
meters-new = Neue Messstelle
meters-edit = Messstelle bearbeiten
meters-empty = Keine Messstellen vorhanden.
meter-name = Name
meter-kind = Art
meter-unit = Einheit
meter-unit-hint = Leer lassen für die Standardeinheit der Art.
meter-serial = Zählernummer
meter-digits = Vorkommastellen
meter-digits-hint = Gesetzt aktiviert die Erkennung eines Zählerüberlaufs.
meter-decimals = Nachkommastellen
meter-interval = Ableseturnus (Tage)
meter-interval-hint = Grundlage für die Fälligkeit der nächsten Ablesung.
meter-max-rate = Maximaler Tagesverbrauch
meter-max-rate-hint = Höhere Tageswerte ergeben einen Hinweis bei der Erfassung.
meter-billing-start = Beginn des Abrechnungsjahres (Monat, Tag)
meter-billing-month = Monat
meter-billing-day = Tag
meter-start-at = Einbau und Startstände am
meter-registers = Register
meter-readings = Ablesungen
meter-devices = Geräte
meter-swap = Zählerwechsel
register-code = Code
register-direction = Richtung
register-start = Startstand
register-add = Register hinzufügen
direction-import = Bezug
direction-export = Einspeisung
kind-electricity = Strom
kind-gas = Gas
kind-water = Wasser
kind-heat = Wärme
kind-vehicle = Fahrzeug
kind-custom = Sonstiges
device-installed = Eingebaut
device-removed = Ausgebaut
device-active = eingebaut

# Ablesungen
readings-empty = Noch keine Ablesungen.
reading-at = Zeitpunkt
reading-value = Stand
reading-register = Register
reading-source = Quelle
reading-note = Notiz
reading-delete-confirm = Diese Ablesung löschen?
source-manual = Formular
source-photo = Foto
source-csv_import = CSV-Import
source-invoice = Rechnung
readings-new = Stand erfassen
readings-last = Letzter Stand { $value } am { $date }
readings-none = Noch kein Stand erfasst
readings-confirm = Trotzdem speichern
readings-warnings = Die Prüfung meldet Auffälligkeiten. Zum Speichern bestätigen.
plaus-decreasing = Stand ist kleiner als der vorherige.
plaus-rate_too_high = Tagesverbrauch liegt über dem Maximum der Messstelle.
plaus-out_of_order = Zeitpunkt liegt vor der letzten Ablesung oder vor dem Einbau.

# Zählerwechsel
swap-title = Zählerwechsel
swap-at = Zeitpunkt des Wechsels
swap-values = Endstände des alten und Startstände des neuen Geräts
swap-new = Neues Gerät
swap-end = Endstand
swap-start = Startstand

# Einstellungen
settings-title = Einstellungen
settings-household = Haushalt
household-name = Name
household-timezone = Zeitzone
household-timezone-hint = IANA-Bezeichnung, z.B. Europe/Berlin.
settings-language = Sprache
settings-new-household = Weiteren Haushalt anlegen
settings-delete-household = Haushalt löschen
settings-delete-hint = Löscht alle Messstellen und Ablesungen dieses Haushalts. Zur Bestätigung den Namen des Haushalts eingeben.
settings-delete-confirm-label = Name des Haushalts

# Anmeldung
login-title = Anmelden
login-oidc = Anmelden
login-dev = Entwicklungs-Login
login-dev-hint = Nur für lokale Entwicklung (auth.dev_login).
login-dev-name = Name
login-submit = Anmelden
login-none = Keine Anmeldung konfiguriert: OIDC oder auth.dev_login einrichten.
login-logged-out = Abgemeldet.
login-failed = Anmeldung fehlgeschlagen.
login-dev-name-required = Name erforderlich.

# Meldungen
flash-meter-created = Messstelle angelegt.
flash-meter-updated = Messstelle gespeichert.
flash-swap-done = Zählerwechsel gespeichert.
flash-readings-saved = Stand gespeichert.
flash-reading-deleted = Ablesung gelöscht.
flash-household-created = Haushalt angelegt.
flash-household-updated = Haushalt gespeichert.
flash-household-deleted = Haushalt gelöscht.
flash-locale-updated = Sprache gespeichert.

# Fehler
error-title = Fehler
error-not-found = Seite oder Eintrag nicht gefunden.
error-forbidden = Keine Berechtigung für diese Aktion.
error-internal = Interner Fehler. Referenz: { $ref }
err-conflict = Die Aktion widerspricht vorhandenen Daten.
err-meter-name_invalid = Name mit 1 bis { $max } Zeichen erforderlich.
err-meter-unit_required = Einheit erforderlich.
err-meter-unit_invalid = Höchstens { $max } Zeichen.
err-meter-interval_invalid = 1 bis 3650 Tage.
err-meter-max_rate_invalid = Wert größer 0 erforderlich.
err-meter-billing_start_invalid = Ungültiger Tag im Jahr.
err-meter-digits_invalid = 1 bis 9 Stellen.
err-meter-decimals_invalid = 0 bis 3 Stellen.
err-meter-serial_invalid = Höchstens { $max } Zeichen.
err-meter-registers_required = Mindestens ein Register erforderlich.
err-meter-register_code_invalid = Code mit 1 bis { $max } Zeichen erforderlich.
err-meter-register_code_duplicate = Code bereits vergeben.
err-meter-name_taken = Name bereits vergeben.
err-meter-serial_taken = Zählernummer bereits einer anderen Messstelle zugeordnet.
err-meter-kind_locked = Art nach der ersten Ablesung nicht änderbar.
err-meter-unit_locked = Einheit nach der ersten Ablesung nicht änderbar.
err-meter-register_locked = Register können nicht entfernt werden.
err-meter-direction_locked = Richtung nach der ersten Ablesung nicht änderbar.
err-meter-no_active_device = Kein eingebautes Gerät vorhanden.
err-reading-negative = Wert darf nicht negativ sein.
err-reading-too_many_decimals = Höchstens { $places } Nachkommastellen.
err-reading-exceeds_digits = Mehr als { $digits } Vorkommastellen.
err-reading-values_required = Mindestens einen Wert eingeben.
err-reading-in_future = Zeitpunkt liegt in der Zukunft.
err-reading-note_too_long = Höchstens { $max } Zeichen.
err-reading-decreasing = Endstand ist kleiner als die letzte Ablesung.
err-reading-duplicate = Für diesen Zeitpunkt ist bereits ein Stand erfasst.
err-reading-part_of_swap = Ablesungen eines Zählerwechsels können nicht gelöscht werden.
err-swap-before_last_reading = Zeitpunkt muss nach der letzten Ablesung liegen.
err-swap-end_required = Endstand erforderlich.
err-swap-start_required = Startstand erforderlich.
err-household-name_invalid = Name mit 1 bis { $max } Zeichen erforderlich.
err-household-delete_confirm_mismatch = Name stimmt nicht überein.
err-settings-locale_invalid = Sprache wird nicht unterstützt.
err-form-number_invalid = Keine gültige Zahl.
err-form-integer_invalid = Keine gültige ganze Zahl.
err-form-datetime_invalid = Kein gültiger Zeitpunkt.
err-form-timezone_invalid = Unbekannte Zeitzone.
err-form-kind_invalid = Ungültige Auswahl.
