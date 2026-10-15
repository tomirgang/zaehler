# ADR-011: Benachrichtigungen per E-Mail

```{adr} Benachrichtigungen nur per E-Mail (SMTP)
:id: ADR_011
:status: accepted
:decides: REQ_NTF_001, REQ_NTF_002, REQ_NTF_004

**Datum:** 2026-10-15
```

## Kontext
Erinnerungen und Abweichungsalarme müssen die Haushaltsmitglieder erreichen.
Jeder zusätzliche Kanal bedeutet einen weiteren Dienst, Zugangsdaten und Pflege.

## Entscheidung
- Einziger Kanal ist **E-Mail über SMTP** (`lettre`, STARTTLS/TLS).
- Ein `Notifier`-Trait kapselt den Versand, damit der Scheduler mit
  Test-Doubles getestet werden kann.

## Alternativen
- *Signal über signal-cli-rest-api*: kein Bedarf neben E-Mail; erfordert einen
  zusätzlichen Dienst und die Pflege einer registrierten Telefonnummer.
- *Apprise* als Proxy für viele Kanäle: zusätzlicher Python-Dienst.
- *Web Push (PWA)*: möglich als spätere Ergänzung ohne Fremddienst.

## Konsequenzen
- Betreiber brauchen einen SMTP-Zugang; ohne SMTP-Konfiguration sind
  Benachrichtigungen deaktiviert.
- Weitere Kanäle lassen sich über den Trait nachrüsten (neues ADR).
