# ADR-024: Outbox für Folgeverarbeitung von Anwendungsfällen

```{adr} Ereignisse über eine Outbox-Tabelle an Hintergrundjobs übergeben
:id: ADR_024
:status: accepted
:decides: REQ_NTF_004, REQ_PV_008, SPEC_JOB_OUTBOX

**Datum:** 2026-10-16
```

## Kontext
Abweichungsalarme ({need}`REQ_NTF_004`) und die Ertragsüberwachung
({need}`REQ_PV_008`) entstehen aus einer neuen Ablesung. Ein E-Mail-Versand im
Request verlängert die Antwortzeit, scheitert unabhängig vom Speichern und
wird bei mehreren Replicas nicht koordiniert. Ein periodischer Vergleich aller
Messstellen erkennt nicht, welche Ablesung neu ist.

## Entscheidung
- Anwendungsfälle schreiben ein Ereignis (`readings_recorded` mit Haushalt,
  Messstelle, Ablesungen) in die Tabelle `outbox`, in derselben Transaktion wie
  die Ablesung.
- Der Job `DispatchOutbox` des Schedulers ({need}`ADR_010`) verarbeitet die
  Einträge auf der Leader-Replica, mit Wiederholung und Fehlerzähler.
- Ein Versandprotokoll (`notification_log`) mit eindeutigem Schlüssel je
  Empfänger und Anlass verhindert doppelte E-Mails, auch wenn ein Eintrag nach
  einem Absturz erneut verarbeitet wird.

## Alternativen
- *Versand im Request*: einfach, aber Fehler und Latenz des SMTP-Servers
  wirken auf die Erfassung; kein Wiederholen.
- *Tokio-Task nach dem Commit*: geht bei Neustart verloren und läuft auf
  jeder Replica.
- *Externe Queue*: zusätzlicher Dienst ({need}`ADR_010`).

## Konsequenzen
- Verzögerung bis zur E-Mail bis zu einem Scheduler-Takt (1 min).
- Die Outbox ist erweiterbar (z.B. Web Push später) ohne Änderung der
  erzeugenden Anwendungsfälle.
- `outbox` und `notification_log` sind Betriebsdaten und nicht Teil des
  Haushaltsexports.
