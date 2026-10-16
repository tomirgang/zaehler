//! Kern der Anwendung (hexagonale Architektur, ADR_022).
//!
//! - [`usecase`]: Anwendungsfälle (Driving Ports), aufgerufen von Driving-Adaptern
//!   wie `zaehler-web`.
//! - [`port`]: Traits für alles Externe (Driven Ports), implementiert von
//!   Adaptern wie `zaehler-db`.
//! - [`Tenant`]: Berechtigungsnachweis; entsteht nur in diesem Crate.
//!
//! Dieses Crate kennt keine Frameworks, keine Datenbank und kein HTTP.

pub mod backup;
pub mod error;
pub mod ids;
pub mod model;
pub mod port;
pub mod tenant;
pub mod usecase;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use error::{AppError, FieldError, FieldErrors, PortError};
pub use tenant::{Role, Tenant};
