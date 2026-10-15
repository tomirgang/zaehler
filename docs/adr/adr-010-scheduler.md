# ADR-010: In-App-Scheduler mit Leader-Lock

```{adr} Hintergrundjobs im Prozess mit Leader-Lock
:id: ADR_010
:status: accepted
:decides: REQ_NTF_001, REQ_OPS_008, SPEC_JOB_SCHEDULER

**Datum:** 2026-10-15
```

## Kontext
Erinnerungen und Alarme müssen periodisch geprüft werden. Bei mehreren Replicas
darf nichts doppelt versendet werden; compose-Setups haben keinen CronJob.

## Entscheidung
Tokio-basierter Scheduler im Serverprozess. Bei PostgreSQL wird er nur auf
der Replica aktiv, die `pg_try_advisory_lock` hält; bei SQLite gibt es nur eine
Instanz. Ausführungen werden in `job_runs` protokolliert (idempotent).

## Alternativen
- *Kubernetes CronJob* mit `zaehler run-jobs`: in k8s nativ verfügbar, in compose nicht vorhanden.
- *Externe Queue* (Redis, NATS): zusätzlicher Dienst für wenige periodische Jobs.

## Konsequenzen
- Lock-Verbindung muss dauerhaft gehalten werden (eigene Verbindung außerhalb des Pools).
