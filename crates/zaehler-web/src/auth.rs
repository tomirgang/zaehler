//! Anmeldung: OIDC (Code Flow mit PKCE) und Entwicklungs-Login
//! (SPEC_TEN_OIDC, SPEC_TEN_DEV_LOGIN).

use askama::Template;
use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    reqwest,
};
use serde::{Deserialize, Serialize};
use tower_sessions::Session;
use zaehler_app::model::Identity;

use crate::{
    extract::{AnonLang, SESSION_CSRF, SESSION_HOUSEHOLD, SESSION_OIDC, SESSION_USER},
    i18n::Lang,
    pages::render,
    security,
    state::WebState,
};

type DiscoveredClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// OIDC-Client nach Discovery.
pub struct OidcClient {
    client: DiscoveredClient,
    http: reqwest::Client,
    issuer: String,
}

impl std::fmt::Debug for OidcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcClient")
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}

impl OidcClient {
    /// Discovery über `{issuer}/.well-known/openid-configuration`.
    // @impl SPEC_TEN_OIDC
    pub async fn discover(
        issuer_url: &str,
        client_id: &str,
        client_secret: &str,
        public_url: &str,
    ) -> Result<Self, String> {
        let http = reqwest::ClientBuilder::new()
            // Weiterleitungen nicht folgen (SSRF-Schutz)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let issuer = IssuerUrl::new(issuer_url.to_owned()).map_err(|e| e.to_string())?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| format!("OIDC-Discovery fehlgeschlagen: {e}"))?;
        let redirect = RedirectUrl::new(format!(
            "{}/auth/callback",
            public_url.trim_end_matches('/')
        ))
        .map_err(|e| e.to_string())?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(client_id.to_owned()),
            Some(ClientSecret::new(client_secret.to_owned())),
        )
        .set_redirect_uri(redirect);
        Ok(Self {
            client,
            http,
            issuer: issuer_url.to_owned(),
        })
    }
}

/// Zustand des Autorisierungsablaufs in der Sitzung.
#[derive(Serialize, Deserialize)]
struct OidcFlow {
    state: String,
    nonce: String,
    pkce_verifier: String,
}

#[derive(Template)]
#[template(path = "pages/login.html")]
struct LoginPage {
    lang: Lang,
    static_prefix: String,
    oidc: bool,
    dev_login: bool,
    message: Option<String>,
}

fn login_page_response(
    state: &WebState,
    lang: Lang,
    message: Option<String>,
    status: StatusCode,
) -> Response {
    let page = LoginPage {
        lang,
        static_prefix: format!("/static/{}", state.cfg.version),
        oidc: state.oidc.is_some(),
        dev_login: state.cfg.dev_login,
        message,
    };
    (status, render(&page)).into_response()
}

#[derive(Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    logged_out: bool,
}

/// Anmeldeseite mit Schaltfläche; ohne automatische Weiterleitung, damit
/// nach dem Abmelden keine sofortige Neuanmeldung über SSO erfolgt.
pub async fn login_page(
    State(state): State<WebState>,
    AnonLang(lang): AnonLang,
    Query(q): Query<LoginQuery>,
) -> Response {
    let message = if q.logged_out {
        Some(lang.t("login-logged-out"))
    } else if state.oidc.is_none() && !state.cfg.dev_login {
        Some(lang.t("login-none"))
    } else {
        None
    };
    login_page_response(&state, lang, message, StatusCode::OK)
}

fn auth_error(state: &WebState, lang: Lang, e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "login failed");
    login_page_response(
        state,
        lang,
        Some(lang.t("login-failed")),
        StatusCode::BAD_REQUEST,
    )
}

/// Startet den Code Flow mit PKCE.
// @impl SPEC_TEN_OIDC
pub async fn oidc_start(
    State(state): State<WebState>,
    AnonLang(lang): AnonLang,
    session: Session,
) -> Response {
    let Some(oidc) = state.oidc.clone() else {
        return login_page_response(
            &state,
            lang,
            Some(lang.t("login-none")),
            StatusCode::NOT_FOUND,
        );
    };
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, csrf_state, nonce) = oidc
        .client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .set_pkce_challenge(challenge)
        .url();
    let flow = OidcFlow {
        state: csrf_state.secret().clone(),
        nonce: nonce.secret().clone(),
        pkce_verifier: verifier.secret().clone(),
    };
    if let Err(e) = session.insert(SESSION_OIDC, flow).await {
        return auth_error(&state, lang, e);
    }
    Redirect::to(url.as_str()).into_response()
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Rückkehr vom Identity Provider.
// @impl SPEC_TEN_OIDC
pub async fn callback(
    State(state): State<WebState>,
    AnonLang(lang): AnonLang,
    session: Session,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(oidc) = state.oidc.clone() else {
        return login_page_response(
            &state,
            lang,
            Some(lang.t("login-none")),
            StatusCode::NOT_FOUND,
        );
    };
    if let Some(error) = q.error {
        return auth_error(&state, lang, format!("Provider meldet: {error}"));
    }
    let flow = match session.remove::<OidcFlow>(SESSION_OIDC).await {
        Ok(Some(f)) => f,
        Ok(None) => return auth_error(&state, lang, "kein Anmeldeablauf in der Sitzung"),
        Err(e) => return auth_error(&state, lang, e),
    };
    let (Some(code), Some(returned_state)) = (q.code, q.state) else {
        return auth_error(&state, lang, "code oder state fehlt");
    };
    if returned_state != flow.state {
        return auth_error(&state, lang, "state stimmt nicht überein");
    }
    let request = match oidc.client.exchange_code(AuthorizationCode::new(code)) {
        Ok(r) => r,
        Err(e) => return auth_error(&state, lang, e),
    };
    let token = match request
        .set_pkce_verifier(PkceCodeVerifier::new(flow.pkce_verifier))
        .request_async(&oidc.http)
        .await
    {
        Ok(t) => t,
        Err(e) => return auth_error(&state, lang, e),
    };
    let Some(id_token) = token.id_token() else {
        return auth_error(&state, lang, "kein ID-Token");
    };
    let claims = match id_token.claims(&oidc.client.id_token_verifier(), &Nonce::new(flow.nonce)) {
        Ok(c) => c,
        Err(e) => return auth_error(&state, lang, e),
    };
    let email = claims.email().map(|e| e.as_str().to_owned());
    let display_name = claims
        .name()
        .and_then(|n| n.get(None))
        .map(|n| n.as_str().to_owned())
        .or_else(|| claims.preferred_username().map(|u| u.as_str().to_owned()))
        .or_else(|| email.clone())
        .unwrap_or_else(|| claims.subject().as_str().to_owned());
    let identity = Identity {
        issuer: claims.issuer().as_str().to_owned(),
        subject: claims.subject().as_str().to_owned(),
        email,
        display_name,
        locale: lang.code().to_owned(),
    };
    complete_sign_in(&state, lang, &session, &identity).await
}

/// Anmeldung abschließen: Sitzung erneuern, Nutzer und Haushalt setzen.
async fn complete_sign_in(
    state: &WebState,
    lang: Lang,
    session: &Session,
    identity: &Identity,
) -> Response {
    let signed = match state.uc.sign_in.execute(identity).await {
        Ok(s) => s,
        Err(e) => return auth_error(state, lang, e),
    };
    let stored = async {
        session.cycle_id().await?;
        session.insert(SESSION_USER, signed.user.id).await?;
        session.insert(SESSION_HOUSEHOLD, signed.active).await?;
        session.insert(SESSION_CSRF, security::new_token()).await?;
        Ok::<_, tower_sessions::session::Error>(())
    }
    .await;
    if let Err(e) = stored {
        return auth_error(state, lang, e);
    }
    tracing::info!(user = %signed.user.id, issuer = %identity.issuer, "signed in");
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/")]).into_response()
}

#[derive(Deserialize)]
pub struct DevLoginQuery {
    name: Option<String>,
}

/// Entwicklungs-Login, nur mit `auth.dev_login = true`.
// @impl SPEC_TEN_DEV_LOGIN
pub async fn dev_login(
    State(state): State<WebState>,
    AnonLang(lang): AnonLang,
    session: Session,
    Query(q): Query<DevLoginQuery>,
) -> Response {
    if !state.cfg.dev_login {
        return StatusCode::NOT_FOUND.into_response();
    }
    let name = q
        .name
        .unwrap_or_default()
        .trim()
        .chars()
        .take(80)
        .collect::<String>();
    if name.is_empty() {
        return login_page_response(
            &state,
            lang,
            Some(lang.t("login-dev-name-required")),
            StatusCode::BAD_REQUEST,
        );
    }
    let identity = Identity {
        issuer: "dev".into(),
        subject: name.clone(),
        email: None,
        display_name: name,
        locale: lang.code().to_owned(),
    };
    complete_sign_in(&state, lang, &session, &identity).await
}

/// Abmelden: Sitzung löschen.
pub async fn logout(session: Session) -> Response {
    if let Err(e) = session.flush().await {
        tracing::warn!(error = %e, "logout failed");
    }
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, "/auth/login?logged_out=true")],
    )
        .into_response()
}
