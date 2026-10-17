//! Export des Haushalts als ZIP mit einer CSV-Datei je Entität
//! (SPEC_IMP_EXPORT_CSV). Grundlage sind die Datensätze des
//! Haushaltsarchivs; Festkommawerte (`*_milli`, `*_micro`) erscheinen als
//! Dezimalzahl ohne Rundung, die Spalte ohne Suffix.

use std::io::{Seek, Write};

use rust_decimal::Decimal;
use serde::{
    Deserializer, Serialize,
    de::{DeserializeOwned, Error as _, Visitor},
};
use serde_json::Value;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    AppError, Role, Tenant,
    backup::{
        AcquisitionRecord, AdjustmentRecord, AssetCostRecord, BackupData, ChargingSessionRecord,
        ConsumptionProfileRecord, ContractRecord, CostPositionRecord, DeviceRecord, DocumentRecord,
        DraftRecord, HouseholdRecord, InstallmentRecord, MembershipRecord, MeterRecord,
        PriceComponentRecord, PvSystemMeterRecord, PvSystemRecord, ReadingRecord, RegisterRecord,
        RevisionRecord, SettlementRecord, VehicleRecord,
    },
    usecase::use_case,
};

use_case!(
    /// Haushalt als ZIP mit CSV-Dateien (Owner).
    ExportHouseholdCsv,
);

/// Felder von `BackupData`, die nicht zum Haushalt gehören (Nutzerkonten,
/// Einladungen, Einstellungen, Tokens; SPEC_BACKUP_HOUSEHOLD).
pub const NOT_EXPORTED: [&str; 4] = [
    "users",
    "invitations",
    "notification_settings",
    "api_tokens",
];

/// Feldnamen eines Datensatzes in Deklarationsreihenfolge, ermittelt über
/// `Deserialize` (auch für leere Tabellen).
struct FieldNames(&'static [&'static str]);

impl<'de> Deserializer<'de> for &mut FieldNames {
    type Error = serde::de::value::Error;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(Self::Error::custom("nur Strukturen"))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Self::Error> {
        self.0 = fields;
        Err(Self::Error::custom("Feldnamen ermittelt"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

fn field_names<T: DeserializeOwned>() -> &'static [&'static str] {
    let mut f = FieldNames(&[]);
    let _ = T::deserialize(&mut f);
    f.0
}

/// Spaltenname und Skala eines Festkommafelds.
fn column(field: &str) -> (&str, Option<u32>) {
    if let Some(name) = field.strip_suffix("_milli") {
        (name, Some(3))
    } else if let Some(name) = field.strip_suffix("_micro") {
        (name, Some(6))
    } else {
        (field, None)
    }
}

fn cell(v: Option<&Value>, scale: Option<u32>) -> String {
    match (v, scale) {
        (None | Some(Value::Null), _) => String::new(),
        (Some(Value::Number(n)), Some(scale)) => n.as_i64().map_or_else(
            || n.to_string(),
            |i| Decimal::new(i, scale).normalize().to_string(),
        ),
        (Some(Value::String(s)), _) => text_cell(s),
        (Some(other), _) => other.to_string(),
    }
}

/// Text, den eine Tabellenkalkulation als Formel ausführen würde, erhält ein
/// führendes `'` (CSV-Injection); Zahlen und Zeitpunkte bleiben unverändert.
fn text_cell(s: &str) -> String {
    let formula = s.starts_with(['=', '+', '-', '@', '\t', '\r']);
    if formula && s.parse::<Decimal>().is_err() {
        format!("'{s}")
    } else {
        s.to_owned()
    }
}

/// CSV einer Tabelle: UTF-8 mit BOM, Komma, Kopfzeile.
fn csv_of<T: Serialize + DeserializeOwned>(rows: &[T]) -> Result<Vec<u8>, AppError> {
    let fields = field_names::<T>();
    let mut out = "\u{feff}".as_bytes().to_vec();
    {
        let mut w = csv::Writer::from_writer(&mut out);
        w.write_record(fields.iter().map(|f| column(f).0))
            .map_err(|e| AppError::Internal(e.to_string()))?;
        for row in rows {
            let value = serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))?;
            w.write_record(fields.iter().map(|f| cell(value.get(f), column(f).1)))
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
        w.flush().map_err(|e| AppError::Internal(e.to_string()))?;
    }
    Ok(out)
}

/// Feld in `BackupData`, Dateiname und Inhalt je Tabelle.
type Table = (&'static str, &'static str, Vec<u8>);

macro_rules! tables {
    ($d:expr, $($field:ident => $file:literal : $ty:ty),+ $(,)?) => {
        vec![$((stringify!($field), $file, csv_of::<$ty>(&$d.$field)?)),+]
    };
}

/// Alle Tabellen des Haushalts.
pub fn tables(d: &BackupData) -> Result<Vec<Table>, AppError> {
    Ok(tables!(d,
        households => "households.csv": HouseholdRecord,
        memberships => "memberships.csv": MembershipRecord,
        meters => "meters.csv": MeterRecord,
        devices => "devices.csv": DeviceRecord,
        registers => "registers.csv": RegisterRecord,
        readings => "readings.csv": ReadingRecord,
        revisions => "reading_revisions.csv": RevisionRecord,
        drafts => "ai_drafts.csv": DraftRecord,
        contracts => "contracts.csv": ContractRecord,
        cost_positions => "cost_positions.csv": CostPositionRecord,
        price_components => "price_components.csv": PriceComponentRecord,
        installments => "installments.csv": InstallmentRecord,
        adjustments => "adjustments.csv": AdjustmentRecord,
        settlements => "settlements.csv": SettlementRecord,
        documents => "documents.csv": DocumentRecord,
        vehicles => "vehicles.csv": VehicleRecord,
        consumption_profiles => "vehicle_consumption_profiles.csv": ConsumptionProfileRecord,
        acquisitions => "vehicle_acquisitions.csv": AcquisitionRecord,
        charging_sessions => "charging_sessions.csv": ChargingSessionRecord,
        asset_costs => "asset_costs.csv": AssetCostRecord,
        pv_systems => "pv_systems.csv": PvSystemRecord,
        pv_system_meters => "pv_system_meters.csv": PvSystemMeterRecord,
    ))
}

impl ExportHouseholdCsv {
    /// ZIP mit einer CSV-Datei je Tabelle des aktiven Haushalts.
    // @impl SPEC_IMP_EXPORT_CSV
    #[tracing::instrument(name = "ExportHouseholdCsv", skip_all)]
    pub async fn execute<W: Write + Seek + Send>(&self, t: &Tenant, out: W) -> Result<W, AppError> {
        t.require(Role::Owner)?;
        let data = self.p.backup.export().await?.household(t.household_id().0);
        let mut zip = ZipWriter::new(out);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let io = |e: zip::result::ZipError| AppError::Internal(e.to_string());
        for (_, file, bytes) in tables(&data)? {
            zip.start_file(file, options).map_err(io)?;
            zip.write_all(&bytes)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
        let out = zip.finish().map_err(io)?;
        tracing::info!(household = %t.household_id(), readings = data.readings.len(), "household exported as csv");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_names_follow_declaration() {
        let f = field_names::<RegisterRecord>();
        assert_eq!(f, ["id", "meter_id", "code", "direction"]);
        assert_eq!(column("value_milli"), ("value", Some(3)));
        assert_eq!(column("amount_micro"), ("amount", Some(6)));
        assert_eq!(
            cell(Some(&Value::from(26_415_250_i64)), Some(3)),
            "26415.25"
        );
        assert_eq!(cell(Some(&Value::from(-1_i64)), Some(6)), "-0.000001");
        assert_eq!(cell(Some(&Value::Null), None), "");
        let text = |s: &str| cell(Some(&Value::from(s)), None);
        assert_eq!(
            text("=HYPERLINK(\"http://x\")"),
            "'=HYPERLINK(\"http://x\")"
        );
        assert_eq!(text("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(text("-12.5"), "-12.5", "Zahl bleibt Zahl");
        assert_eq!(text("Keller"), "Keller");
    }

    /// Jede Tabelle des Haushaltsarchivs ist als CSV enthalten.
    // @test SPEC_IMP_EXPORT_CSV
    #[test]
    fn every_household_table_is_exported() {
        let Value::Object(all) = serde_json::to_value(BackupData::default()).unwrap() else {
            panic!("Objekt erwartet");
        };
        let mut expected: Vec<&str> = all
            .keys()
            .map(String::as_str)
            .filter(|k| !NOT_EXPORTED.contains(k))
            .collect();
        expected.sort_unstable();
        let mut exported: Vec<&str> = tables(&BackupData::default())
            .unwrap()
            .iter()
            .map(|(field, ..)| *field)
            .collect();
        exported.sort_unstable();
        assert_eq!(exported, expected);
        for (_, file, bytes) in tables(&BackupData::default()).unwrap() {
            assert!(bytes.starts_with("\u{feff}".as_bytes()), "{file}: BOM");
            assert!(bytes.len() > 3, "{file}: Kopfzeile");
        }
    }

    // @test SPEC_IMP_EXPORT_CSV
    #[tokio::test]
    async fn csv_zip_keeps_decimals_and_household() {
        use rust_decimal_macros::dec;
        use zaehler_domain::meter::{Direction, MeterKind};

        use crate::usecase::contract_tests::{berlin, meter_input, record, setup};

        let (uc, t, _) = setup().await;
        let mut m = meter_input(
            "Wasser, Keller \"Nord\"",
            MeterKind::Water,
            &[("main", Direction::Import)],
            berlin(2025, 12, 1),
        );
        m.decimal_places = 3;
        let meter = uc.create_meter.execute(&t, m).await.unwrap();
        record(&uc, &t, meter, berlin(2026, 1, 10), dec!(1234.567)).await;
        let out = uc
            .export_household_csv
            .execute(&t, std::io::Cursor::new(Vec::new()))
            .await
            .unwrap();
        // Probe mit Tabellenkalkulation und Python `csv` (WP_V1_22)
        if let Ok(path) = std::env::var("ZAEHLER_CSV_SAMPLE") {
            std::fs::write(path, out.get_ref()).unwrap();
        }
        let mut zip = zip::ZipArchive::new(out).unwrap();
        let mut read = |name: &str| {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut s).unwrap();
            s
        };
        let readings = read("readings.csv");
        let mut r = csv::Reader::from_reader(readings.trim_start_matches('\u{feff}').as_bytes());
        let headers = r.headers().unwrap().clone();
        let value = headers.iter().position(|h| h == "value").unwrap();
        let values: Vec<String> = r.records().map(|x| x.unwrap()[value].to_owned()).collect();
        assert!(values.contains(&"1234.567".to_owned()), "{values:?}");
        assert!(headers.iter().any(|h| h == "read_at"));
        let meters = read("meters.csv");
        assert!(
            meters.contains("\"Wasser, Keller \"\"Nord\"\"\""),
            "{meters}"
        );
        assert!(zip.by_name("users.csv").is_err());
    }
}
