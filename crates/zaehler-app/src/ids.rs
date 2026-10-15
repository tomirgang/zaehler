//! Typisierte IDs; verhindern Verwechslungen zwischen Entitäten.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    )+};
}

id_type!(
    /// Haushalt (Mandant)
    HouseholdId,
    /// Nutzer
    UserId,
    /// Messstelle
    MeterId,
    /// Physisches Zählergerät
    DeviceId,
    /// Register (Zählwerk)
    RegisterId,
    /// Ablesung
    ReadingId,
);
