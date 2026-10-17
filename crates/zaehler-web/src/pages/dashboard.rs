//! Übersicht: Karten je aktiver Messstelle (SPEC_UI_DASHBOARD).

use askama::Template;
use axum::{extract::State, response::Response};
use chrono::Datelike;
use zaehler_app::usecase::analysis::DashboardCard;
use zaehler_domain::{analysis::PeriodForecast, schedule::Due};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::{Lang, format},
    pages::{
        analysis::{TrafficView, traffic_view},
        render,
    },
    state::WebState,
};

pub struct Card {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub last: Option<String>,
    pub values: Vec<String>,
    pub last_month: Option<String>,
    pub vs_previous: Option<String>,
    pub forecast: Option<String>,
    pub due: Option<String>,
    pub overdue: bool,
    /// Fällig oder überfällig
    pub due_now: bool,
    /// Ampel des letzten vollständigen Monats (SPEC_ANA_TRAFFIC_LIGHT)
    pub traffic: Option<TrafficView>,
}

#[derive(Template)]
#[template(path = "pages/dashboard.html")]
struct DashboardPage {
    ctx: PageCtx,
    cards: Vec<Card>,
    /// PV-Karten je Hausanschluss (SPEC_PV_CHARTS)
    pv: Vec<PvCardView>,
}

// @impl SPEC_UI_DASHBOARD
fn card(lang: Lang, tz: chrono_tz::Tz, c: &DashboardCard) -> Card {
    let data = &c.meter.data;
    let unit = &data.unit;
    let places = c.last_values.first().map_or(1, |v| v.decimal_places);
    // @impl SPEC_RDG_SCHEDULE
    let due = c.due.map(|d| {
        let key = match d {
            Due::Upcoming(_) => "dashboard-due",
            Due::Due(_) => "dashboard-due-today",
            Due::Overdue(_) => "dashboard-overdue",
        };
        lang.ta(key, &[("date", format::date(lang, d.date()))])
    });
    Card {
        id: c.meter.id.to_string(),
        name: data.name.clone(),
        kind: lang.t(&format!("kind-{}", data.kind.as_str())),
        last: c.last_reading_at.map(|t| format::datetime_in(lang, t, tz)),
        values: c
            .last_values
            .iter()
            .map(|v| {
                format!(
                    "{} {} {unit}",
                    v.register,
                    format::decimal(lang, v.value, v.decimal_places)
                )
            })
            .collect(),
        last_month: c.last_month.and_then(|m| {
            Some(lang.ta(
                "dashboard-last-month",
                &[
                    ("month", format::month(lang, m.month.start)),
                    (
                        "value",
                        format!("{} {unit}", format::decimal(lang, m.value?, places)),
                    ),
                ],
            ))
        }),
        vs_previous: c.previous_month.as_ref().map(|p| {
            let diff = p.rel_diff_percent.map_or_else(
                || format!("{} {unit}", format::signed(lang, p.abs_diff, places)),
                |r| format!("{} %", format::signed(lang, r, 1)),
            );
            lang.ta("dashboard-vs-previous", &[("diff", diff)])
        }),
        forecast: match &c.forecast {
            PeriodForecast::Projected(f) => Some(lang.ta(
                "dashboard-forecast",
                &[
                    ("year", f.period.start.year().to_string()),
                    (
                        "value",
                        format!("{} {unit}", format::decimal(lang, f.projected_total, 0)),
                    ),
                    ("coverage", format::percent(lang, f.coverage, 0)),
                ],
            )),
            _ => None,
        },
        due,
        overdue: c.overdue(),
        due_now: c.due.is_some_and(Due::needs_reading),
        traffic: c
            .traffic
            .as_ref()
            .and_then(|e| traffic_view(lang, e, unit, places)),
    }
}

/// PV-Karte eines Hausanschlusses.
pub struct PvCardView {
    pub id: String,
    pub name: String,
    pub month: String,
    pub generation: String,
    pub autarky: String,
    pub ratio: String,
    pub status: &'static str,
    pub status_label: String,
}

// @impl SPEC_PV_CHARTS
fn pv_card(lang: Lang, c: &zaehler_app::usecase::pv::PvCard) -> PvCardView {
    use zaehler_domain::expectation::Status;
    let status = match c.status {
        Some(Status::Green) => "green",
        Some(Status::Yellow) => "yellow",
        Some(Status::Red) => "red",
        None => "",
    };
    let pct = |v: Option<rust_decimal::Decimal>| {
        v.map(|v| crate::i18n::format::percent(lang, v, 0))
            .unwrap_or_default()
    };
    PvCardView {
        id: c.grid.to_string(),
        name: c.name.clone(),
        month: c
            .month
            .map(|m| crate::i18n::format::month(lang, m.start))
            .unwrap_or_default(),
        generation: c
            .generation
            .map(|g| format!("{} kWh", crate::i18n::format::decimal(lang, g, 0)))
            .unwrap_or_default(),
        autarky: pct(c.autarky),
        ratio: pct(c.self_consumption_ratio),
        status,
        status_label: if status.is_empty() {
            String::new()
        } else {
            lang.t(&format!("traffic-{status}"))
        },
    }
}

// @impl SPEC_UI_DASHBOARD
pub async fn show(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    let cards = state
        .uc
        .dashboard
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let tz = ctx.page.tz;
    let pv = state
        .uc
        .pv_dashboard
        .execute(&ctx.tenant)
        .await
        .map_err(|e| ctx.fail(e))?;
    Ok(render(&DashboardPage {
        cards: cards.iter().map(|c| card(lang, tz, c)).collect(),
        pv: pv.iter().map(|c| pv_card(lang, c)).collect(),
        ctx: ctx.page,
    }))
}
