//! Fachlogik der Zähler-App.
//!
//! Dieses Crate ist bewusst frei von IO (keine DB, kein HTTP), damit sämtliche
//! Berechnungen deterministisch und ohne Infrastruktur testbar sind. Kommentare der Form
//! `// @impl SPEC_…` bzw. `// @test SPEC_…` werden von der Doku-Toolchain
//! (`docs/_ext/codetrace.py`) eingesammelt und als Trace-Links gerendert.

pub mod analysis;
pub mod assets;
pub mod calendar;
pub mod comparison;
pub mod consumption;
pub mod cost;
pub mod expectation;
pub mod forecast;
pub mod meter;
pub mod period;
pub mod reading;
pub mod schedule;
pub mod series;
pub mod tariff;
pub mod vehicle;

pub use rust_decimal::Decimal;
