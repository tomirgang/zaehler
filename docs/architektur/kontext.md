# Kontext

```{mermaid}
flowchart LR
  user([Haushaltsmitglied<br/>Browser / PWA])
  subgraph cluster[Kubernetes / docker compose]
    app[zaehler<br/>Axum-Server]
    db[(PostgreSQL CNPG<br/>oder SQLite)]
    store[(Objektspeicher<br/>S3 oder Volume)]
  end
  idp[OIDC-Provider<br/>Keycloak / Authentik]
  or[OpenRouter<br/>Vision-LLM]
  smtp[SMTP-Server]
  prom[Prometheus / OTel]

  user -- HTTPS --> app
  user -. Login .-> idp
  app -- OIDC --> idp
  app --> db
  app --> store
  app -- HTTPS, Bild + Prompt --> or
  app -- SMTP --> smtp
  prom -- scrape :9090 --> app
```

| Nachbarsystem | Zweck | Optional |
|---|---|---|
| OIDC-Provider | Authentifizierung | nein (außer Dev-Modus) |
| OpenRouter | Erkennung von Zählerständen, Belegen, Rechnungen | ja |
| SMTP | E-Mail-Benachrichtigungen | ja |
| Prometheus / OTel-Collector | Metriken, Traces | ja |
