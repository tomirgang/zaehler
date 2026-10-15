//! Internationalisierung (SPEC_I18N, ADR_015): Fluent-Texte, Sprachauswahl,
//! Formatierung und Parsen lokalisierter Eingaben.

pub mod format;
pub mod parse;

use std::{borrow::Cow, collections::HashMap};

use fluent_templates::{
    LanguageIdentifier, Loader, fluent_bundle::FluentValue, langid, static_loader,
};
use zaehler_app::FieldError;

static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "de",
        // keine Unicode-Isolationszeichen um Platzhalter (stören in Attributen)
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

static DE: LanguageIdentifier = langid!("de");
static EN: LanguageIdentifier = langid!("en");

/// Sprache einer Antwort.
// @impl SPEC_I18N
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

impl Lang {
    /// Sprache aus einem gespeicherten Code; unbekannt ergibt Deutsch.
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        if code.eq_ignore_ascii_case("en") {
            Self::En
        } else {
            Self::De
        }
    }

    /// Erste unterstützte Sprache aus `Accept-Language`, sonst Deutsch.
    #[must_use]
    pub fn from_accept_language(header: Option<&str>) -> Self {
        let Some(header) = header else {
            return Self::De;
        };
        let mut entries: Vec<(f32, &str)> = header
            .split(',')
            .filter_map(|part| {
                let mut it = part.trim().split(';');
                let tag = it.next()?.trim();
                let q = it
                    .find_map(|p| p.trim().strip_prefix("q="))
                    .and_then(|q| q.parse::<f32>().ok())
                    .unwrap_or(1.0);
                Some((q, tag))
            })
            .collect();
        entries.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, tag) in entries {
            let primary = tag.split('-').next().unwrap_or_default();
            if primary.eq_ignore_ascii_case("de") {
                return Self::De;
            }
            if primary.eq_ignore_ascii_case("en") {
                return Self::En;
            }
        }
        Self::De
    }

    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::De => "de",
            Self::En => "en",
        }
    }

    fn id(self) -> &'static LanguageIdentifier {
        match self {
            Self::De => &DE,
            Self::En => &EN,
        }
    }

    /// Text zu einem Fluent-Schlüssel.
    #[must_use]
    pub fn t(&self, key: &str) -> String {
        LOCALES.lookup(self.id(), key)
    }

    /// Text mit Argumenten.
    #[must_use]
    pub fn ta(&self, key: &str, args: &[(&str, String)]) -> String {
        let map: HashMap<Cow<'static, str>, FluentValue> = args
            .iter()
            .map(|(k, v)| (Cow::Owned((*k).to_owned()), FluentValue::from(v.clone())))
            .collect();
        LOCALES.lookup_with_args(self.id(), key, &map)
    }

    /// Text zu einem Fehlercode des Kerns, z.B. `meter.name_invalid`.
    #[must_use]
    pub fn code_text(&self, code: &str) -> String {
        self.t(&format!("err-{}", code.replace('.', "-")))
    }

    /// Text zu einem Feldfehler inklusive Argumenten.
    #[must_use]
    pub fn err(&self, e: &FieldError) -> String {
        let key = format!("err-{}", e.code.replace('.', "-"));
        let args: Vec<(&str, String)> = e.args.iter().map(|(k, v)| (*k, v.clone())).collect();
        self.ta(&key, &args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Vec<String> {
        let mut keys: Vec<String> = text
            .lines()
            .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
            .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim().to_owned()))
            .collect();
        keys.sort();
        keys
    }

    // @test SPEC_I18N
    #[test]
    fn german_and_english_have_the_same_keys() {
        let de = keys(include_str!("../../locales/de/main.ftl"));
        let en = keys(include_str!("../../locales/en/main.ftl"));
        let missing_en: Vec<_> = de.iter().filter(|k| !en.contains(k)).collect();
        let missing_de: Vec<_> = en.iter().filter(|k| !de.contains(k)).collect();
        assert!(
            missing_en.is_empty() && missing_de.is_empty(),
            "fehlt en: {missing_en:?}, fehlt de: {missing_de:?}"
        );
        assert!(de.len() > 100);
    }

    // @test SPEC_I18N
    #[test]
    fn language_selection_and_lookup() {
        assert_eq!(
            Lang::from_accept_language(Some("fr-FR, en-US;q=0.8, de;q=0.5")),
            Lang::En
        );
        assert_eq!(Lang::from_accept_language(Some("de-AT,de;q=0.9")), Lang::De);
        assert_eq!(Lang::from_accept_language(Some("fr")), Lang::De);
        assert_eq!(Lang::from_accept_language(None), Lang::De);
        assert_eq!(Lang::De.t("nav-meters"), "Messstellen");
        assert_eq!(Lang::En.t("nav-meters"), "Meters");
        let e = FieldError {
            field: "x".into(),
            code: "reading.too_many_decimals",
            args: vec![("places", "2".into())],
        };
        assert_eq!(Lang::De.err(&e), "Höchstens 2 Nachkommastellen.");
    }
}
