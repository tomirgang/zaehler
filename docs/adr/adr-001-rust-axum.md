# ADR-001: Rust mit Axum und Tokio

```{adr} Rust mit Axum und Tokio als Web-Stack
:id: ADR_001
:status: accepted
:decides: REQ_OPS_001, REQ_NFR_006

**Datum:** 2026-10-15
```

## Kontext
Zielumgebungen sind Kubernetes und Heimserver mit begrenztem Arbeitsspeicher;
die Auslieferung erfolgt als ein Binary.

## Entscheidung
Rust (Edition 2024) mit **Axum 0.8** auf **Tokio**, Middleware über
`tower-http`. CLI-Unterbefehle (`serve`, `migrate`, `config`) mit `clap`.

## Alternativen
- *Actix Web*: eigenes Middleware-System statt Tower.
- *Loco.rs*: Rails-ähnliches Framework mit festgelegten Konventionen für ORM, Jobs
  und Mailer, die über den Bedarf hinausgehen.

## Konsequenzen
- Image-Größe 41,6 MB einschließlich distroless-Basis.
- Längere Compile-Zeiten; CI nutzt Caching.
