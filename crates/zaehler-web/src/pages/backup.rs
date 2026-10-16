//! Haushaltsarchiv herunterladen und als neuen Haushalt importieren
//! (SPEC_BACKUP_HOUSEHOLD).

use std::io::{Seek, SeekFrom, Write};

use axum::{
    body::Body,
    extract::{Multipart, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use tokio_util::io::ReaderStream;
use zaehler_app::AppError;

use crate::{
    extract::{Ctx, SESSION_HOUSEHOLD},
    pages::see_other,
    security::constant_time_eq,
    state::WebState,
};

fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(e.to_string())
}

/// Dateiname aus dem Haushaltsnamen: Kleinbuchstaben, Ziffern, Bindestriche.
fn file_name(household: &str, date: chrono::NaiveDate) -> String {
    let mut slug = String::new();
    for c in household.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'ä' => "ae".into(),
            'ö' => "oe".into(),
            'ü' => "ue".into(),
            'ß' => "ss".into(),
            c if c.is_ascii_alphanumeric() => c.to_string(),
            _ => "-".into(),
        };
        if !(c == "-" && (slug.is_empty() || slug.ends_with('-'))) {
            slug.push_str(&c);
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "haushalt" } else { slug };
    format!("zaehler-{slug}-{}.zip", date.format("%Y-%m-%d"))
}

/// `GET /settings/export`: Archiv des aktiven Haushalts (nur Owner).
// @impl SPEC_BACKUP_HOUSEHOLD
pub async fn export(State(state): State<WebState>, ctx: Ctx) -> Result<Response, Response> {
    let file = tempfile::tempfile().map_err(|e| ctx.fail(internal(e)))?;
    let (mut file, _) = state
        .uc
        .export_household
        .execute(&ctx.tenant, file)
        .await
        .map_err(|e| ctx.fail(e))?;
    let len = file
        .seek(SeekFrom::End(0))
        .and_then(|len| file.seek(SeekFrom::Start(0)).map(|_| len))
        .map_err(|e| ctx.fail(internal(e)))?;
    let name = ctx
        .page
        .households
        .iter()
        .find(|h| h.id == ctx.page.active)
        .map_or("haushalt", |h| h.name.as_str());
    let today = chrono::Utc::now().with_timezone(&ctx.page.tz).date_naive();
    let body = Body::from_stream(ReaderStream::new(tokio::fs::File::from_std(file)));
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (header::CONTENT_LENGTH, len.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", file_name(name, today)),
            ),
            (header::CACHE_CONTROL, "private, no-store".to_owned()),
        ],
        body,
    )
        .into_response())
}

async fn back_with(ctx: &Ctx, key: &str) -> Response {
    ctx.flash(key).await;
    see_other("/settings")
}

/// `POST /settings/import` (Multipart, Feld `_csrf`): Archiv als neuen
/// Haushalt einspielen und zu ihm wechseln.
// @impl SPEC_BACKUP_HOUSEHOLD, SPEC_SEC_UPLOADS, SPEC_SEC_SESSION_CSRF
pub async fn import(
    State(state): State<WebState>,
    ctx: Ctx,
    mut multipart: Multipart,
) -> Result<Response, Response> {
    let mut token = None;
    let mut archive = None;
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Err(upload_error(&ctx, e.status()).await),
        };
        match field.name() {
            Some("_csrf") => token = field.text().await.ok(),
            Some("file") => {
                // Archive können groß sein: in eine temporäre Datei statt in den Speicher
                let mut file = tempfile::tempfile().map_err(|e| ctx.fail(internal(e)))?;
                let mut written = 0_usize;
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => {
                            written += chunk.len();
                            file.write_all(&chunk).map_err(|e| ctx.fail(internal(e)))?;
                        }
                        Ok(None) => break,
                        Err(e) => return Err(upload_error(&ctx, e.status()).await),
                    }
                }
                if written > 0 {
                    archive = Some(file);
                }
            }
            _ => {}
        }
    }
    if !token.is_some_and(|t| constant_time_eq(&t, &ctx.page.csrf)) {
        return Err((StatusCode::FORBIDDEN, "CSRF-Prüfung fehlgeschlagen").into_response());
    }
    let Some(mut file) = archive else {
        return Ok(back_with(&ctx, "err-backup-file_required").await);
    };
    file.seek(SeekFrom::Start(0))
        .map_err(|e| ctx.fail(internal(e)))?;
    match state.uc.import_household.execute(&ctx.tenant, file).await {
        Ok((id, _)) => {
            if let Err(e) = ctx.session.insert(SESSION_HOUSEHOLD, id).await {
                tracing::warn!(error = %e, "household not switched");
            }
            Ok(back_with(&ctx, "flash-household-imported").await)
        }
        Err(AppError::Validation(errors)) => {
            let code = errors.0.first().map_or("backup.invalid", |e| e.code);
            Ok(back_with(&ctx, &format!("err-{}", code.replace('.', "-"))).await)
        }
        Err(e) => Err(ctx.fail(e)),
    }
}

/// Body zu groß (`upload.max_backup_bytes`) oder fehlerhaftes Multipart:
/// zurück zu den Einstellungen mit Hinweis.
async fn upload_error(ctx: &Ctx, status: StatusCode) -> Response {
    let key = if status == StatusCode::PAYLOAD_TOO_LARGE {
        "err-backup-too_large"
    } else {
        "err-backup-invalid"
    };
    back_with(ctx, key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_ascii() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        assert_eq!(
            file_name("Haus Müller / Süd", d),
            "zaehler-haus-mueller-sued-2026-10-16.zip"
        );
        assert_eq!(file_name("\"; rm", d), "zaehler-rm-2026-10-16.zip");
        assert_eq!(file_name("", d), "zaehler-haushalt-2026-10-16.zip");
    }
}
