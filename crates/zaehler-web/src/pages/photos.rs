//! Ansichtsseite für Fotos von Ablesungen und Dokumenten (SPEC_UI_PHOTO_VIEW).

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use serde::Deserialize;
use zaehler_app::{
    AppError,
    ids::{DocumentId, ReadingId},
    model::{DocumentKind, DocumentType},
};

use crate::{
    extract::{Ctx, PageCtx},
    i18n::format,
    pages::render,
    state::WebState,
};

#[derive(Deserialize)]
pub struct ViewQuery {
    back: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/photo.html")]
struct PhotoPage {
    ctx: PageCtx,
    title: String,
    /// Übergeordnetes Glied der Breadcrumb-Leiste (Pfad, Bezeichnung)
    parent: Option<(String, String)>,
    /// Bild-Route; `None` bei PDF
    image: Option<String>,
    original: String,
    back: String,
    facts: Vec<(String, String)>,
}

/// Rücksprungadresse nur als relativer Pfad derselben Anwendung, sonst
/// `None` (Schutz vor Open Redirect).
// @impl SPEC_UI_PHOTO_VIEW
#[must_use]
pub fn safe_back(back: Option<&str>) -> Option<String> {
    let b = back?;
    let ok = b.len() <= 512
        && b.starts_with('/')
        && !b.starts_with("//")
        && !b.starts_with("/\\")
        && !b.chars().any(|c| c.is_control() || c == '\\');
    ok.then(|| b.to_owned())
}

// @impl SPEC_UI_PHOTO_VIEW
pub async fn reading(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<ViewQuery>,
) -> Result<Response, Response> {
    let id: ReadingId = id.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    let rc = state
        .uc
        .get_reading
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    if rc.reading.photo_key.is_none() {
        return Err(ctx.fail(AppError::NotFound));
    }
    let lang = ctx.page.lang;
    let agg = &rc.detail.aggregate;
    let r = &rc.reading;
    let meter_path = format!("/meters/{}", agg.meter.id);
    let mut facts = vec![
        (lang.t("photo-meter"), agg.meter.data.name.clone()),
        (
            lang.t("photo-read-at"),
            format::datetime_in(lang, r.read_at, ctx.page.tz),
        ),
        (
            lang.t("photo-value"),
            format!(
                "{} {}",
                format::decimal_plain(lang, r.value),
                agg.meter.data.unit
            ),
        ),
        (
            lang.t("reading-source"),
            lang.t(&format!("source-{}", r.source.as_str())),
        ),
    ];
    if agg.registers.len() > 1
        && let Some(reg) = agg.register(r.register_id)
    {
        facts.insert(1, (lang.t("photo-register"), reg.data.code.clone()));
    }
    let src = format!("/readings/{id}/photo");
    Ok(render(&PhotoPage {
        title: lang.t("photo-title"),
        parent: Some((meter_path.clone(), agg.meter.data.name.clone())),
        image: Some(src.clone()),
        original: src,
        back: safe_back(q.back.as_deref()).unwrap_or(meter_path),
        facts,
        ctx: ctx.page.clone(),
    }))
}

// @impl SPEC_UI_PHOTO_VIEW
pub async fn document(
    State(state): State<WebState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<ViewQuery>,
) -> Result<Response, Response> {
    let id: DocumentId = id.parse().map_err(|_| ctx.fail(AppError::NotFound))?;
    let doc = state
        .uc
        .get_document
        .execute(&ctx.tenant, id)
        .await
        .map_err(|e| ctx.fail(e))?;
    let lang = ctx.page.lang;
    let kind = match doc.kind {
        DocumentKind::Invoice => lang.t("document-kind-invoice"),
        DocumentKind::Receipt => lang.t("document-receipt"),
    };
    let mut facts = vec![
        (lang.t("document-kind"), kind),
        (lang.t("document-name"), doc.original_name.clone()),
        (
            lang.t("document-uploaded"),
            format::datetime_in(lang, doc.created_at, ctx.page.tz),
        ),
    ];
    if let Some(r) = &doc.receipt {
        if let Some(d) = r.date {
            facts.push((lang.t("photo-date"), format::date(lang, d)));
        }
        if let Some(a) = r.total_amount {
            facts.push((lang.t("photo-amount"), super::contracts::money(lang, a)));
        }
        if let Some(v) = &r.vendor {
            facts.push((lang.t("photo-vendor"), v.clone()));
        }
    }
    let file = format!("/documents/{id}/file");
    Ok(render(&PhotoPage {
        title: doc.original_name.clone(),
        parent: None,
        image: (doc.media_type == DocumentType::Jpeg).then(|| file.clone()),
        original: file,
        back: safe_back(q.back.as_deref()).unwrap_or_else(|| format!("/documents/{id}")),
        facts,
        ctx: ctx.page.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::safe_back;

    // @test SPEC_UI_PHOTO_VIEW
    #[test]
    fn back_accepts_only_relative_paths() {
        assert_eq!(
            safe_back(Some("/meters/1?x=2")).as_deref(),
            Some("/meters/1?x=2")
        );
        for bad in [
            "//evil.example",
            "https://evil.example",
            "/\\evil.example",
            "/a\\b",
            "/a\nb",
            "meters",
            "",
        ] {
            assert_eq!(safe_back(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(safe_back(Some(&format!("/{}", "a".repeat(600)))), None);
        assert_eq!(safe_back(None), None);
    }
}
