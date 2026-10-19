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
    /// Keine aktive Messstelle: leerer Zustand
    empty: bool,
    setup_has_draft: bool,
    /// Karten nach Typ gruppiert (SPEC_UI_DASHBOARD_GROUPS)
    groups: Vec<DashboardGroup>,
}

/// Abschnitt der Übersicht je Typ: Messstellenart oder PV.
pub struct DashboardGroup {
    pub key: &'static str,
    pub title: String,
    pub cards: Vec<Card>,
    /// PV-Karten je Hausanschluss (SPEC_PV_CHARTS), nur in der Gruppe `pv`
    pub pv: Vec<PvCardView>,
}

/// Reihenfolge der Gruppen; PV folgt auf Strom.
const GROUP_ORDER: [&str; 7] = [
    "electricity",
    "pv",
    "heat",
    "gas",
    "water",
    "vehicle",
    "custom",
];

/// Gruppiert Karten nach Typ in fester Reihenfolge und sortiert je Gruppe
/// nach Name ohne Beachtung der Groß- und Kleinschreibung; leere Gruppen
/// entfallen.
// @impl SPEC_UI_DASHBOARD_GROUPS
fn groups(
    lang: Lang,
    mut cards: Vec<(&'static str, Card)>,
    mut pv: Vec<PvCardView>,
) -> Vec<DashboardGroup> {
    let key = |name: &str, id: &str| (name.to_lowercase(), id.to_owned());
    pv.sort_by_cached_key(|p| key(&p.name, &p.id));
    cards.sort_by_cached_key(|(_, c)| key(&c.name, &c.id));
    let mut out = Vec::new();
    for k in GROUP_ORDER {
        let group = if k == "pv" {
            DashboardGroup {
                key: k,
                title: lang.t("pv-title"),
                cards: Vec::new(),
                pv: std::mem::take(&mut pv),
            }
        } else {
            let (members, rest): (Vec<_>, Vec<_>) =
                cards.into_iter().partition(|(kind, _)| *kind == k);
            cards = rest;
            DashboardGroup {
                key: k,
                title: lang.t(&format!("kind-{k}")),
                cards: members.into_iter().map(|(_, c)| c).collect(),
                pv: Vec::new(),
            }
        };
        if !group.cards.is_empty() || !group.pv.is_empty() {
            out.push(group);
        }
    }
    out
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
    let setup_has_draft = if ctx.page.is_owner {
        state
            .uc
            .get_setup_draft
            .execute(&ctx.tenant)
            .await
            .map_err(|e| ctx.fail(e))?
            .is_some()
    } else {
        false
    };
    Ok(render(&DashboardPage {
        empty: cards.is_empty(),
        setup_has_draft,
        groups: groups(
            lang,
            cards
                .iter()
                .map(|c| (c.meter.data.kind.as_str(), card(lang, tz, c)))
                .collect(),
            pv.iter().map(|c| pv_card(lang, c)).collect(),
        ),
        ctx: ctx.page,
    }))
}

#[cfg(test)]
mod tests {
    use super::{Card, PvCardView, groups};
    use crate::i18n::Lang;

    fn card(id: &str, name: &str) -> Card {
        Card {
            id: id.into(),
            name: name.into(),
            kind: String::new(),
            last: None,
            values: Vec::new(),
            last_month: None,
            vs_previous: None,
            forecast: None,
            due: None,
            overdue: false,
            due_now: false,
            traffic: None,
        }
    }

    fn pv(id: &str, name: &str) -> PvCardView {
        PvCardView {
            id: id.into(),
            name: name.into(),
            month: String::new(),
            generation: String::new(),
            autarky: String::new(),
            ratio: String::new(),
            status: "",
            status_label: String::new(),
        }
    }

    // @test SPEC_UI_DASHBOARD_GROUPS
    #[test]
    fn cards_are_grouped_by_type_and_sorted_by_name() {
        let g = groups(
            Lang::De,
            vec![
                ("vehicle", card("1", "Kombi")),
                ("electricity", card("2", "Wallbox")),
                ("custom", card("3", "Pellets")),
                ("electricity", card("4", "anna")),
                ("water", card("5", "Garten")),
                ("electricity", card("6", "Bernd")),
            ],
            vec![pv("7", "Dach")],
        );
        let keys: Vec<_> = g.iter().map(|g| g.key).collect();
        assert_eq!(keys, ["electricity", "pv", "water", "vehicle", "custom"]);
        let names: Vec<_> = g[0].cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["anna", "Bernd", "Wallbox"]);
        assert_eq!(g[0].title, "Strom");
        assert_eq!(g[1].pv.len(), 1);
        assert_eq!(g[4].title, "Sonstiges");
    }

    // @test SPEC_UI_DASHBOARD_GROUPS
    #[test]
    fn empty_groups_are_left_out() {
        assert!(groups(Lang::De, Vec::new(), Vec::new()).is_empty());
        let g = groups(Lang::De, Vec::new(), vec![pv("1", "Dach")]);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].key, "pv");
    }
}
