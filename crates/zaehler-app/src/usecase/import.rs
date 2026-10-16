//! CSV-Import von Zählerständen (SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT).
//!
//! Ablauf: `UploadCsv` legt die Datei unter `tmp/` ab, `PreviewImport`
//! liefert Spaltenvorschlag und Prüfergebnis je Zeile ohne Speicherung,
//! `CommitImport` prüft erneut in einer Transaktion und speichert.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rust_decimal::Decimal;
use zaehler_domain::{
    calendar::Calendar,
    reading::{Plausibility, Reading as DomainReading, ReadingSource, check_in_sequence},
};

use crate::{
    AppError, FieldError, FieldErrors, Role, Tenant,
    ids::BlobKind,
    ids::{BlobKey, DeviceId, MeterId, ReadingId, RegisterId},
    model::{Device, MeterAggregate, Reading},
    usecase::{Ports, reading::MAX_NOTE, use_case, validate},
};

/// Höchstzahl Datenzeilen je Datei.
pub const MAX_ROWS: usize = 20_000;
/// Zeilen der Vorschau im Zuordnungsschritt.
pub const SAMPLE_ROWS: usize = 5;

use_case!(
    /// CSV-Datei prüfen und temporär ablegen.
    UploadCsv,
    /// Spaltenvorschlag und Prüfergebnis je Zeile, ohne Speicherung.
    PreviewImport,
    /// Geprüfte Zeilen in einer Transaktion speichern.
    CommitImport,
);

/// Trennzeichen der Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    Semicolon,
    Tab,
    Comma,
}

impl Delimiter {
    #[must_use]
    pub fn byte(self) -> u8 {
        match self {
            Self::Semicolon => b';',
            Self::Tab => b'\t',
            Self::Comma => b',',
        }
    }
}

/// Häufigstes Trennzeichen der ersten Zeile; bei Gleichstand gilt die
/// Reihenfolge `;`, Tab, `,`.
// @impl SPEC_IMP_CSV_FORMAT
#[must_use]
pub fn detect_delimiter(first_line: &str) -> Delimiter {
    [Delimiter::Semicolon, Delimiter::Tab, Delimiter::Comma]
        .into_iter()
        .rev()
        .max_by_key(|d| first_line.bytes().filter(|b| *b == d.byte()).count())
        .unwrap_or(Delimiter::Semicolon)
}

/// Datumsformat einer Spalte; Uhrzeit optional in derselben Spalte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateFormat {
    /// `TT.MM.JJJJ`
    DayMonthYear,
    /// `JJJJ-MM-TT`
    YearMonthDay,
    /// ISO 8601 mit Zeit, optional mit Offset
    Iso8601,
}

impl DateFormat {
    pub const ALL: [Self; 3] = [Self::DayMonthYear, Self::YearMonthDay, Self::Iso8601];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DayMonthYear => "dmy",
            Self::YearMonthDay => "ymd",
            Self::Iso8601 => "iso",
        }
    }
}

impl std::str::FromStr for DateFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|f| f.as_str() == s).ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalSeparator {
    Comma,
    Point,
}

impl DecimalSeparator {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Comma => "comma",
            Self::Point => "point",
        }
    }
}

impl std::str::FromStr for DecimalSeparator {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "comma" => Ok(Self::Comma),
            "point" => Ok(Self::Point),
            _ => Err(()),
        }
    }
}

/// Zuordnung der Spalten (Index ab 0) und Formate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportMapping {
    pub meter_id: MeterId,
    pub has_header: bool,
    pub date: usize,
    pub time: Option<usize>,
    pub value: usize,
    pub register: Option<usize>,
    pub serial: Option<usize>,
    pub note: Option<usize>,
    pub date_format: DateFormat,
    pub decimal: DecimalSeparator,
    /// Register für Zeilen ohne Registerangabe
    pub default_register: Option<RegisterId>,
}

/// Vorschlag für die Zuordnung aus Überschriften und ersten Zeilen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingSuggestion {
    pub has_header: bool,
    pub date: usize,
    pub time: Option<usize>,
    pub value: usize,
    pub register: Option<usize>,
    pub serial: Option<usize>,
    pub note: Option<usize>,
    pub date_format: DateFormat,
    pub decimal: DecimalSeparator,
}

/// Aufbau der hochgeladenen Datei.
#[derive(Debug, Clone)]
pub struct ImportSheet {
    pub delimiter: Delimiter,
    /// Inhalt der ersten Zeile (Überschriften, falls vorhanden)
    pub first: Vec<String>,
    pub columns: usize,
    /// Erste Datenzeilen gemäß Vorschlag
    pub sample: Vec<Vec<String>>,
    /// Anzahl Datenzeilen gemäß Vorschlag
    pub rows: usize,
    pub suggestion: MappingSuggestion,
}

/// Ergebnis der Prüfung einer Zeile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowStatus {
    Ok,
    /// Plausibilitätshinweis; Speichern nur nach Bestätigung
    Warning(Plausibility),
    /// Gleiches Register, gleicher Zeitpunkt, gleicher Wert vorhanden
    Duplicate,
    Error(FieldError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowResult {
    /// Zeilennummer in der Datei (ab 1)
    pub line: usize,
    pub at: Option<DateTime<Utc>>,
    pub register: Option<String>,
    pub value: Option<Decimal>,
    pub status: RowStatus,
}

#[derive(Debug, Clone, Default)]
pub struct ImportPreview {
    pub rows: Vec<RowResult>,
    pub ok: usize,
    pub warnings: usize,
    pub duplicates: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub inserted: usize,
    pub duplicates: usize,
    /// Zeilen mit Hinweis, die ohne Bestätigung nicht gespeichert wurden
    pub warnings_skipped: usize,
    pub errors: usize,
}

/// Zeile der Datei mit Zeilennummer.
type Row = (usize, Vec<String>);

fn row_error(code: &'static str) -> FieldError {
    FieldError {
        field: "row".into(),
        code,
        args: Vec::new(),
    }
}

/// UTF-8 ohne BOM; Binärdaten (NUL) und leere Dateien werden abgelehnt.
// @impl SPEC_SEC_UPLOADS, SPEC_IMP_CSV_FORMAT
fn decode(bytes: &[u8]) -> Result<&str, AppError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text =
        std::str::from_utf8(bytes).map_err(|_| AppError::field("file", "import.not_utf8"))?;
    if text.contains('\0') {
        return Err(AppError::field("file", "import.not_text"));
    }
    if text.trim().is_empty() {
        return Err(AppError::field("file", "import.empty"));
    }
    Ok(text)
}

/// Zerlegt die Datei; leere Zeilen entfallen.
// @impl SPEC_IMP_CSV_FORMAT
fn parse_rows(text: &str) -> Result<(Delimiter, Vec<Row>), AppError> {
    let first_line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    let delimiter = detect_delimiter(first_line);
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter.byte())
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let bytes = text.as_bytes();
    // Zeilennummern fortlaufend aus dem Byte-Offset; `csv` zählt leere Zeilen
    // nicht mit und meldet den Beginn davor liegender Leerzeilen
    let (mut scanned, mut line) = (0_usize, 1_usize);
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|_| AppError::field("file", "import.parse"))?;
        let mut offset = record
            .position()
            .and_then(|p| usize::try_from(p.byte()).ok())
            .unwrap_or(scanned)
            .clamp(scanned, bytes.len());
        line += text[scanned..offset].matches('\n').count();
        while offset < bytes.len() && matches!(bytes[offset], b'\n' | b'\r') {
            line += usize::from(bytes[offset] == b'\n');
            offset += 1;
        }
        scanned = offset;
        let cells: Vec<String> = record.iter().map(|c| c.trim().to_owned()).collect();
        if cells.iter().all(String::is_empty) {
            continue;
        }
        rows.push((line, cells));
        if rows.len() > MAX_ROWS + 1 {
            return Err(AppError::Validation(FieldErrors(vec![FieldError {
                field: "file".into(),
                code: "import.too_many_rows",
                args: vec![("max", MAX_ROWS.to_string())],
            }])));
        }
    }
    Ok((delimiter, rows))
}

/// Datum mit optionaler Uhrzeit; `Ok(local)` oder exakter Zeitpunkt (ISO mit Offset).
enum Parsed {
    Local(NaiveDateTime),
    Exact(DateTime<Utc>),
}

fn parse_local(s: &str, formats: &[&str], date_format: &str) -> Option<NaiveDateTime> {
    formats
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .or_else(|| {
            NaiveDate::parse_from_str(s, date_format)
                .ok()
                .map(|d| d.and_time(NaiveTime::MIN))
        })
}

fn parse_date(s: &str, format: DateFormat) -> Option<Parsed> {
    let s = s.trim();
    match format {
        DateFormat::DayMonthYear => {
            parse_local(s, &["%d.%m.%Y %H:%M:%S", "%d.%m.%Y %H:%M"], "%d.%m.%Y").map(Parsed::Local)
        }
        DateFormat::YearMonthDay => {
            parse_local(s, &["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"], "%Y-%m-%d").map(Parsed::Local)
        }
        DateFormat::Iso8601 => DateTime::parse_from_rfc3339(s)
            .map(|t| Parsed::Exact(t.to_utc()))
            .ok()
            .or_else(|| {
                parse_local(s, &["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"], "%Y-%m-%d")
                    .map(Parsed::Local)
            }),
    }
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M"))
        .ok()
}

/// Zahl mit dem gewählten Dezimaltrenner; Tausendertrennzeichen und
/// Vorzeichen werden abgelehnt.
// @impl SPEC_IMP_CSV_FORMAT
fn parse_value(s: &str, sep: DecimalSeparator) -> Option<Decimal> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let (dec, other) = match sep {
        DecimalSeparator::Comma => (',', '.'),
        DecimalSeparator::Point => ('.', ','),
    };
    if s.is_empty() || s.contains(other) || s.matches(dec).count() > 1 {
        return None;
    }
    let n = s.replace(dec, ".");
    if !n.chars().all(|c| c.is_ascii_digit() || c == '.') || n.starts_with('.') {
        return None;
    }
    n.parse().ok()
}

fn is_date(s: &str) -> bool {
    DateFormat::ALL
        .into_iter()
        .any(|f| parse_date(s, f).is_some())
}

fn find_column(header: &[String], keywords: &[&str], taken: &[usize]) -> Option<usize> {
    header.iter().enumerate().find_map(|(i, h)| {
        let h = h.to_lowercase();
        (!taken.contains(&i) && keywords.iter().any(|k| h.contains(k))).then_some(i)
    })
}

/// Zuordnungsvorschlag: Überschrift, wenn die erste Zeile kein Datum enthält;
/// Spalten über Schlüsselwörter (de/en), sonst über die erste Datenzeile.
// @impl SPEC_IMP_CSV_FORMAT
fn suggest(rows: &[Row]) -> MappingSuggestion {
    let first = rows.first().map(|r| r.1.as_slice()).unwrap_or_default();
    let has_header = !first.iter().any(|c| is_date(c));
    let body: Vec<&Vec<String>> = rows
        .iter()
        .skip(usize::from(has_header))
        .take(SAMPLE_ROWS)
        .map(|r| &r.1)
        .collect();
    let sample = body.first().map(|r| r.as_slice()).unwrap_or_default();
    let header: &[String] = if has_header { first } else { &[] };

    let date = find_column(header, &["datum", "date", "zeitpunkt", "tag", "day"], &[])
        .or_else(|| sample.iter().position(|c| is_date(c)))
        .unwrap_or(0);
    let value = find_column(
        header,
        &["zählerstand", "stand", "wert", "value", "reading", "menge"],
        &[date],
    )
    .or_else(|| {
        sample.iter().enumerate().find_map(|(i, c)| {
            (i != date
                && (parse_value(c, DecimalSeparator::Comma).is_some()
                    || parse_value(c, DecimalSeparator::Point).is_some()))
            .then_some(i)
        })
    })
    .unwrap_or(usize::from(date == 0));
    let register = find_column(
        header,
        &["register", "zählwerk", "tarif", "obis"],
        &[date, value],
    );
    let serial = find_column(
        header,
        &["zählernummer", "seriennummer", "serial", "nummer", "number"],
        &[date, value],
    );
    let note = find_column(
        header,
        &["notiz", "bemerkung", "kommentar", "note", "comment"],
        &[date, value],
    );
    let time = find_column(header, &["uhrzeit", "zeit", "time"], &[date, value]);
    let date_sample = sample.get(date).map_or("", String::as_str);
    let date_format = [
        DateFormat::Iso8601,
        DateFormat::DayMonthYear,
        DateFormat::YearMonthDay,
    ]
    .into_iter()
    .find(|f| {
        parse_date(date_sample, *f).is_some()
            && (*f != DateFormat::Iso8601 || date_sample.contains('T'))
    })
    .unwrap_or(DateFormat::DayMonthYear);
    let decimal = if body
        .iter()
        .any(|r| r.get(value).is_some_and(|c| c.contains(',')))
    {
        DecimalSeparator::Comma
    } else {
        DecimalSeparator::Point
    };
    MappingSuggestion {
        has_header,
        date,
        time,
        value,
        register,
        serial,
        note,
        date_format,
        decimal,
    }
}

fn normalize_code(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Gerät für eine Ablesung: über die Zählernummer oder das zum Zeitpunkt
/// eingebaute Gerät. Vor dem Einbau des ersten Geräts gilt das erste Gerät.
fn device_for<'a>(
    agg: &'a MeterAggregate,
    serial: Option<&str>,
    at: DateTime<Utc>,
) -> Result<&'a Device, FieldError> {
    let first = agg
        .devices
        .first()
        .ok_or_else(|| row_error("import.no_device"))?;
    let covers = |d: &Device| {
        (d.data.installed_at <= at || d.id == first.id) && d.removed_at.is_none_or(|r| at <= r)
    };
    match serial
        .map(validate::normalize_serial)
        .filter(|s| !s.is_empty())
    {
        Some(s) => {
            let device = agg
                .devices
                .iter()
                .find(|d| {
                    d.data
                        .serial_number
                        .as_deref()
                        .is_some_and(|x| validate::normalize_serial(x) == s)
                })
                .ok_or_else(|| row_error("import.serial_unknown"))?;
            if covers(device) {
                Ok(device)
            } else {
                Err(row_error("import.outside_device"))
            }
        }
        None => agg
            .devices
            .iter()
            .find(|d| covers(d))
            .ok_or_else(|| row_error("import.no_device")),
    }
}

/// Geprüfte Zeilen und die daraus entstehenden Ablesungen.
struct Plan {
    results: Vec<RowResult>,
    /// Index in `results` und Ablesung (nur `Ok` und `Warning`)
    readings: Vec<(usize, Reading)>,
}

/// Prüfung aller Zeilen gegen Messstelle und vorhandene Ablesungen.
// @impl SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT, SPEC_RDG_PLAUSIBILITY
#[allow(clippy::too_many_lines)] // eine Prüfung je Spalte in fester Reihenfolge
fn evaluate(
    p: &Ports,
    t: &Tenant,
    rows: &[Row],
    m: &ImportMapping,
    agg: &MeterAggregate,
    existing: &[Reading],
    cal: Calendar,
) -> Plan {
    let now = p.clock.now();
    let cell = |r: &Row, i: Option<usize>| -> String {
        i.and_then(|i| r.1.get(i)).cloned().unwrap_or_default()
    };
    let mut results = Vec::new();
    let mut readings: Vec<(usize, Reading)> = Vec::new();
    let existing_values: HashMap<(RegisterId, DeviceId, DateTime<Utc>), Decimal> = existing
        .iter()
        .map(|r| ((r.register_id, r.device_id, r.read_at), r.value))
        .collect();
    let mut seen: HashMap<(RegisterId, DeviceId, DateTime<Utc>), Decimal> = HashMap::new();

    for row in rows.iter().skip(usize::from(m.has_header)) {
        let mut result = RowResult {
            line: row.0,
            at: None,
            register: None,
            value: None,
            status: RowStatus::Ok,
        };
        let checked = (|| -> Result<Reading, FieldError> {
            let at = match parse_date(&cell(row, Some(m.date)), m.date_format) {
                None => return Err(row_error("import.date_invalid")),
                Some(Parsed::Exact(at)) => at,
                Some(Parsed::Local(local)) => {
                    let time = cell(row, m.time);
                    let local = if time.is_empty() {
                        local
                    } else {
                        let t =
                            parse_time(&time).ok_or_else(|| row_error("import.time_invalid"))?;
                        local.date().and_time(t)
                    };
                    cal.to_utc(local)
                }
            };
            result.at = Some(at);
            if at > now + Duration::minutes(5) {
                return Err(row_error("reading.in_future"));
            }
            let value = parse_value(&cell(row, Some(m.value)), m.decimal)
                .ok_or_else(|| row_error("import.value_invalid"))?;
            result.value = Some(value);
            let code = cell(row, m.register);
            let register = if code.is_empty() {
                m.default_register
                    .or_else(|| (agg.registers.len() == 1).then(|| agg.registers[0].id))
                    .and_then(|id| agg.register(id))
                    .ok_or_else(|| row_error("import.register_missing"))?
            } else {
                agg.registers
                    .iter()
                    .find(|r| normalize_code(&r.data.code) == normalize_code(&code))
                    .ok_or_else(|| row_error("import.register_unknown"))?
            };
            result.register = Some(register.data.code.clone());
            let serial = cell(row, m.serial);
            let device = device_for(agg, Some(serial.as_str()).filter(|s| !s.is_empty()), at)?;
            let mut e = FieldErrors::default();
            validate::meter_value(
                &mut e,
                "value",
                value,
                device.data.decimal_places,
                device.data.rollover_digits,
            );
            if let Some(err) = e.0.into_iter().next() {
                return Err(err);
            }
            let note = Some(cell(row, m.note)).filter(|n| !n.is_empty());
            if note.as_ref().is_some_and(|n| n.chars().count() > MAX_NOTE) {
                return Err(FieldError {
                    field: "row".into(),
                    code: "reading.note_too_long",
                    args: vec![("max", MAX_NOTE.to_string())],
                });
            }
            Ok(Reading {
                id: ReadingId(p.ids.new_uuid()),
                register_id: register.id,
                device_id: device.id,
                read_at: at,
                value,
                source: ReadingSource::CsvImport,
                note,
                photo_key: None,
                ai_confidence: None,
                created_by: Some(t.user_id()),
            })
        })();
        match checked {
            Err(e) => result.status = RowStatus::Error(e),
            Ok(reading) => {
                let key = (reading.register_id, reading.device_id, reading.read_at);
                match existing_values.get(&key).or_else(|| seen.get(&key)) {
                    Some(v) if *v == reading.value => result.status = RowStatus::Duplicate,
                    Some(_) => result.status = RowStatus::Error(row_error("import.conflict")),
                    None => {
                        seen.insert(key, reading.value);
                        readings.push((results.len(), reading));
                    }
                }
            }
        }
        results.push(result);
    }
    mark_plausibility(agg, existing, &readings, &mut results);
    Plan { results, readings }
}

/// Plausibilität je Register und Gerät gegen Vorgänger und Nachfolger aus
/// vorhandenen und importierten Ablesungen.
// @impl SPEC_IMP_CSV, SPEC_RDG_PLAUSIBILITY
fn mark_plausibility(
    agg: &MeterAggregate,
    existing: &[Reading],
    planned: &[(usize, Reading)],
    results: &mut [RowResult],
) {
    let groups: HashSet<(RegisterId, DeviceId)> = planned
        .iter()
        .map(|(_, r)| (r.register_id, r.device_id))
        .collect();
    let max_rate = agg.meter.data.max_daily_rate;
    for (register, device) in groups {
        let rollover = agg.device(device).and_then(|d| d.data.rollover_digits);
        let domain = |r: &Reading| DomainReading {
            at: r.read_at,
            value: r.value,
            device_id: device.0,
            rollover_digits: rollover,
        };
        // (Index in `results` bei importierten Zeilen, Ablesung)
        let mut merged: Vec<(Option<usize>, DomainReading)> = existing
            .iter()
            .filter(|r| r.register_id == register && r.device_id == device)
            .map(|r| (None, domain(r)))
            .chain(
                planned
                    .iter()
                    .filter(|(_, r)| r.register_id == register && r.device_id == device)
                    .map(|(i, r)| (Some(*i), domain(r))),
            )
            .collect();
        merged.sort_by_key(|(_, r)| r.at);
        for k in 0..merged.len() {
            let Some(index) = merged[k].0 else { continue };
            let current = &merged[k].1;
            let before = k.checked_sub(1).map(|j| &merged[j].1);
            let next = merged.get(k + 1).map(|(_, r)| r);
            let p = check_in_sequence(before, current, next, max_rate);
            if p != Plausibility::Ok {
                results[index].status = RowStatus::Warning(p);
            }
        }
    }
}

fn summarize(rows: Vec<RowResult>) -> ImportPreview {
    let mut out = ImportPreview::default();
    for r in &rows {
        match r.status {
            RowStatus::Ok => out.ok += 1,
            RowStatus::Warning(_) => out.warnings += 1,
            RowStatus::Duplicate => out.duplicates += 1,
            RowStatus::Error(_) => out.errors += 1,
        }
    }
    out.rows = rows;
    out
}

/// Schlüssel muss eine temporäre Datei des eigenen Haushalts sein.
fn own_tmp(t: &Tenant, key: &BlobKey) -> Result<(), AppError> {
    if key.household() == t.household_id() && key.kind() == BlobKind::Tmp {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

async fn load_rows(
    p: &Ports,
    t: &Tenant,
    key: &BlobKey,
) -> Result<(Delimiter, Vec<Row>), AppError> {
    own_tmp(t, key)?;
    let bytes = p.blobs.get(key).await.map_err(AppError::from_storage)?;
    parse_rows(decode(&bytes)?)
}

fn check_mapping(m: &ImportMapping, agg: &MeterAggregate, columns: usize) -> Result<(), AppError> {
    let mut e = FieldErrors::default();
    for (field, col) in [
        ("col_date", Some(m.date)),
        ("col_value", Some(m.value)),
        ("col_time", m.time),
        ("col_register", m.register),
        ("col_serial", m.serial),
        ("col_note", m.note),
    ] {
        if col.is_some_and(|c| c >= columns) {
            e.add(field, "import.column_invalid");
        }
    }
    if m.date == m.value {
        e.add("col_value", "import.column_invalid");
    }
    if m.default_register
        .is_some_and(|r| agg.register(r).is_none())
    {
        e.add("default_register", "import.register_unknown");
    }
    e.into_result()
}

impl UploadCsv {
    /// Prüft Kodierung und Aufbau und legt die Datei unter `tmp/` ab
    /// (Housekeeping entfernt sie spätestens nach 24 Stunden).
    // @impl SPEC_IMP_CSV, SPEC_SEC_UPLOADS
    #[tracing::instrument(name = "UploadCsv", skip_all)]
    pub async fn execute(&self, t: &Tenant, bytes: Bytes) -> Result<BlobKey, AppError> {
        t.require(Role::Member)?;
        let (_, rows) = parse_rows(decode(&bytes)?)?;
        if rows.len() < 2 && suggest(&rows).has_header {
            return Err(AppError::field("file", "import.empty"));
        }
        let key = BlobKey::tmp(t.household_id(), self.p.ids.new_uuid());
        self.p
            .blobs
            .put(&key, bytes)
            .await
            .map_err(AppError::from_storage)?;
        Ok(key)
    }
}

impl PreviewImport {
    /// Aufbau der Datei mit Zuordnungsvorschlag.
    // @impl SPEC_IMP_CSV_FORMAT
    #[tracing::instrument(name = "PreviewImport::sheet", skip_all)]
    pub async fn sheet(&self, t: &Tenant, key: &BlobKey) -> Result<ImportSheet, AppError> {
        t.require(Role::Member)?;
        let (delimiter, rows) = load_rows(&self.p, t, key).await?;
        let suggestion = suggest(&rows);
        let skip = usize::from(suggestion.has_header);
        Ok(ImportSheet {
            delimiter,
            first: rows.first().map(|r| r.1.clone()).unwrap_or_default(),
            columns: rows.iter().map(|r| r.1.len()).max().unwrap_or(0),
            sample: rows
                .iter()
                .skip(skip)
                .take(SAMPLE_ROWS)
                .map(|r| r.1.clone())
                .collect(),
            rows: rows.len().saturating_sub(skip),
            suggestion,
        })
    }

    /// Prüfergebnis je Zeile, ohne Speicherung.
    // @impl SPEC_IMP_CSV
    #[tracing::instrument(name = "PreviewImport", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        key: &BlobKey,
        mapping: &ImportMapping,
    ) -> Result<ImportPreview, AppError> {
        t.require(Role::Member)?;
        let (_, rows) = load_rows(&self.p, t, key).await?;
        let mut uow = self.p.uow.begin().await?;
        let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
        let agg = uow
            .meters()
            .get(t, mapping.meter_id)
            .await?
            .ok_or(AppError::NotFound)?;
        check_mapping(
            mapping,
            &agg,
            rows.iter().map(|r| r.1.len()).max().unwrap_or(0),
        )?;
        let existing = uow.readings().list_for_meter(t, mapping.meter_id).await?;
        drop(uow);
        let plan = evaluate(&self.p, t, &rows, mapping, &agg, &existing, cal);
        Ok(summarize(plan.results))
    }
}

impl CommitImport {
    /// Prüft erneut in einer Transaktion und speichert Zeilen `ok` sowie mit
    /// `confirm_warnings` die Zeilen mit Hinweis; liegt eine Ablesung vor dem
    /// Einbau des ersten Geräts, wird der Einbau vorverlegt. Danach wird die
    /// Datei gelöscht.
    // @impl SPEC_IMP_CSV, SPEC_IMP_CSV_FORMAT
    #[tracing::instrument(name = "CommitImport", skip_all)]
    pub async fn execute(
        &self,
        t: &Tenant,
        key: &BlobKey,
        mapping: &ImportMapping,
        confirm_warnings: bool,
    ) -> Result<ImportReport, AppError> {
        t.require(Role::Member)?;
        let (_, rows) = load_rows(&self.p, t, key).await?;
        let mut uow = self.p.uow.begin().await?;
        let cal = Calendar::new(uow.households().get(t).await?.data.timezone);
        let agg = uow
            .meters()
            .get(t, mapping.meter_id)
            .await?
            .ok_or(AppError::NotFound)?;
        // @impl SPEC_MTR_ARCHIVE
        if agg.meter.archived {
            return Err(AppError::field("meter_id", "meter.archived"));
        }
        check_mapping(
            mapping,
            &agg,
            rows.iter().map(|r| r.1.len()).max().unwrap_or(0),
        )?;
        let existing = uow.readings().list_for_meter(t, mapping.meter_id).await?;
        let plan = evaluate(&self.p, t, &rows, mapping, &agg, &existing, cal);

        let mut report = ImportReport::default();
        let mut insert = Vec::new();
        for (index, reading) in plan.readings {
            match plan.results[index].status {
                RowStatus::Warning(_) if !confirm_warnings => report.warnings_skipped += 1,
                _ => insert.push(reading),
            }
        }
        for r in &plan.results {
            match r.status {
                RowStatus::Duplicate => report.duplicates += 1,
                RowStatus::Error(_) => report.errors += 1,
                _ => {}
            }
        }
        if let Some(first) = agg.devices.first()
            && let Some(earliest) = insert
                .iter()
                .filter(|r| r.device_id == first.id)
                .map(|r| r.read_at)
                .min()
            && earliest < first.data.installed_at
        {
            let mut data = first.data.clone();
            data.installed_at = earliest;
            uow.meters().update_device(t, first.id, &data).await?;
        }
        if !insert.is_empty() {
            uow.readings()
                .insert(t, &insert)
                .await
                .map_err(|e| match e {
                    crate::PortError::Conflict(_) => AppError::Conflict("reading.duplicate"),
                    other => other.into(),
                })?;
        }
        uow.commit().await?;
        report.inserted = insert.len();
        for (result, n) in [
            ("ok", report.inserted),
            ("duplicate", report.duplicates),
            ("warning_skipped", report.warnings_skipped),
            ("error", report.errors),
        ] {
            metrics::counter!("zaehler_import_rows_total", "result" => result).increment(n as u64);
        }
        metrics::counter!("zaehler_readings_created_total", "source" => ReadingSource::CsvImport.as_str())
            .increment(report.inserted as u64);
        if let Err(e) = self.p.blobs.delete(key).await {
            tracing::warn!(error = %e, "import file not deleted");
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn rows(text: &str) -> Vec<Row> {
        parse_rows(decode(text.as_bytes()).unwrap()).unwrap().1
    }

    // @test SPEC_IMP_CSV_FORMAT
    #[test]
    fn delimiter_bom_and_german_format() {
        let text =
            "\u{feff}Datum;Zählerstand;Notiz\n01.01.2026;1.234,5;Start\n\n01.02.2026;1.300,0;\n";
        assert!(matches!(decode(text.as_bytes()), Ok(t) if t.starts_with("Datum")));
        let (d, r) = parse_rows(decode(text.as_bytes()).unwrap()).unwrap();
        assert_eq!(d, Delimiter::Semicolon);
        assert_eq!(r.len(), 3, "leere Zeile entfällt");
        assert_eq!(r[2].0, 4, "Zeilennummer der Datei");
        let s = suggest(&r);
        assert!(s.has_header);
        assert_eq!((s.date, s.value, s.note), (0, 1, Some(2)));
        assert_eq!(s.date_format, DateFormat::DayMonthYear);
        assert_eq!(s.decimal, DecimalSeparator::Comma);
        // Tausendertrennzeichen werden abgelehnt
        assert_eq!(parse_value("1.234,5", DecimalSeparator::Comma), None);
        assert_eq!(
            parse_value("1234,5", DecimalSeparator::Comma),
            Some(dec!(1234.5))
        );
        assert_eq!(
            parse_value("1234.5", DecimalSeparator::Point),
            Some(dec!(1234.5))
        );
        assert_eq!(parse_value("-1", DecimalSeparator::Point), None);
    }

    // @test SPEC_IMP_CSV_FORMAT
    #[test]
    fn iso_tab_comma_and_files_without_header() {
        let tab = rows("2026-01-01T08:00:00+01:00\t100.5\tHT\n2026-02-01T08:00\t110\tHT\n");
        let s = suggest(&tab);
        assert!(!s.has_header);
        assert_eq!(
            (s.date, s.value, s.date_format),
            (0, 1, DateFormat::Iso8601)
        );
        assert_eq!(s.decimal, DecimalSeparator::Point);
        assert!(matches!(
            parse_date("2026-01-01T08:00:00+01:00", DateFormat::Iso8601),
            Some(Parsed::Exact(t)) if t.to_rfc3339() == "2026-01-01T07:00:00+00:00"
        ));
        let comma = rows("date,time,reading,register\n2026-03-01,07:30,15.5,NT\n");
        assert_eq!(
            detect_delimiter("date,time,reading,register"),
            Delimiter::Comma
        );
        let s = suggest(&comma);
        assert_eq!(
            (s.date, s.time, s.value, s.register),
            (0, Some(1), 2, Some(3))
        );
        assert_eq!(s.date_format, DateFormat::YearMonthDay);
        assert!(parse_date("15.01.2026", DateFormat::YearMonthDay).is_none());
    }

    // @test SPEC_SEC_UPLOADS, SPEC_IMP_CSV_FORMAT
    #[test]
    fn binary_empty_and_oversized_files_are_rejected() {
        let code = |r: Result<&str, AppError>| match r {
            Err(AppError::Validation(e)) => e.0[0].code,
            _ => "ok",
        };
        assert_eq!(code(decode(b"\xFF\xD8\xFF\xE0jpeg")), "import.not_utf8");
        assert_eq!(code(decode(b"a;b\0c")), "import.not_text");
        assert_eq!(code(decode(b"\xEF\xBB\xBF \n")), "import.empty");
        let mut big = String::new();
        for i in 0..=MAX_ROWS + 1 {
            big.push_str("01.01.2026;");
            big.push_str(&i.to_string());
            big.push('\n');
        }
        assert!(matches!(
            parse_rows(&big),
            Err(AppError::Validation(e)) if e.0[0].code == "import.too_many_rows"
        ));
    }
}
