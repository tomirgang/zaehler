//! Datentypen an den Port-Grenzen. Adapter bilden ihre eigenen Typen
//! (SeaORM-Entities, Formulare) auf diese Typen ab.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use zaehler_domain::{
    meter::{Direction, MeterKind},
    reading::ReadingSource,
};

use crate::{
    ids::{DeviceId, HouseholdId, MeterId, ReadingId, RegisterId, UserId},
    tenant::Role,
};

/// Identität aus dem Identity Provider (bzw. Dev-Login).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub display_name: String,
    /// Bevorzugte Sprache für neu angelegte Nutzer (`de` oder `en`).
    pub locale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub display_name: String,
    pub email: Option<String>,
    pub locale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseholdSummary {
    pub id: HouseholdId,
    pub name: String,
    pub timezone: Tz,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseholdData {
    pub name: String,
    pub timezone: Tz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Household {
    pub id: HouseholdId,
    pub data: HouseholdData,
}

/// Monat und Tag, z.B. Beginn des Abrechnungsjahres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthDay {
    pub month: u8,
    pub day: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterData {
    pub name: String,
    pub kind: MeterKind,
    pub unit: String,
    pub reading_interval_days: Option<u16>,
    pub max_daily_rate: Option<Decimal>,
    pub billing_start: Option<MonthDay>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meter {
    pub id: MeterId,
    pub data: MeterData,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceData {
    pub serial_number: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub rollover_digits: Option<u8>,
    pub decimal_places: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub data: DeviceData,
    pub removed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterData {
    pub code: String,
    pub direction: Direction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
    pub id: RegisterId,
    pub data: RegisterData,
}

/// Messstelle mit Geräten (nach Einbau sortiert) und Registern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterAggregate {
    pub meter: Meter,
    pub devices: Vec<Device>,
    pub registers: Vec<Register>,
}

impl MeterAggregate {
    /// Aktuell eingebautes Gerät.
    #[must_use]
    pub fn active_device(&self) -> Option<&Device> {
        self.devices.iter().rev().find(|d| d.removed_at.is_none())
    }

    #[must_use]
    pub fn device(&self, id: DeviceId) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    #[must_use]
    pub fn register(&self, id: RegisterId) -> Option<&Register> {
        self.registers.iter().find(|r| r.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterSummary {
    pub meter: Meter,
    pub last_reading_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub id: ReadingId,
    pub register_id: RegisterId,
    pub device_id: DeviceId,
    pub read_at: DateTime<Utc>,
    pub value: Decimal,
    pub source: ReadingSource,
    pub note: Option<String>,
    pub photo_key: Option<String>,
    pub created_by: Option<UserId>,
}
