//! Übersicht: Messstellen mit letztem Stand und Fälligkeit (Platzhalter für
//! das Dashboard aus WP_MVP_09).

use askama::Template;
use axum::{extract::State, response::Response};
use chrono::{Duration, Utc};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::format,
    pages::render,
    state::WebState,
};

pub struct Card {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub last: Option<String>,
    pub due: Option<String>,
    pub overdue: bool,
}

#[derive(Template)]
#[template(path = "pages/dashboard.html")]
struct DashboardPage {
    ctx: PageCtx,
    cards: Vec<Card>,
}

pub async fn show(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    let meters = state
        .uc
        .list_meters
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let tz = ctx.page.tz;
    let now = Utc::now();
    let mut cards: Vec<Card> = meters
        .into_iter()
        .filter(|m| !m.meter.archived)
        .map(|m| {
            let due_at = m
                .last_reading_at
                .zip(m.meter.data.reading_interval_days)
                .map(|(last, days)| last + Duration::days(i64::from(days)));
            let overdue = due_at.is_some_and(|d| d < now);
            let due = due_at.map(|d| {
                let date = format::date(lang, d.with_timezone(&tz).date_naive());
                lang.ta(
                    if overdue {
                        "dashboard-overdue"
                    } else {
                        "dashboard-due"
                    },
                    &[("date", date)],
                )
            });
            Card {
                id: m.meter.id.to_string(),
                name: m.meter.data.name.clone(),
                kind: lang.t(&format!("kind-{}", m.meter.data.kind.as_str())),
                last: m.last_reading_at.map(|t| format::datetime_in(lang, t, tz)),
                due,
                overdue,
            }
        })
        .collect();
    cards.sort_by(|a, b| {
        b.overdue
            .cmp(&a.overdue)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(render(&DashboardPage {
        ctx: ctx.page,
        cards,
    }))
}
