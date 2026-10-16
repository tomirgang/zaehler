//! Driven-Adapter für den Port `ImageNormalizer` (SPEC_IMG_PROCESSING,
//! ADR_017): Format über Magic Bytes prüfen, Aufnahmezeit auslesen,
//! dekodieren, EXIF-Ausrichtung anwenden, verkleinern und als JPEG ohne
//! Metadaten kodieren.

use std::io::Cursor;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{FixedOffset, NaiveDate};
use image::{
    DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits,
    codecs::jpeg::JpegEncoder, imageops::FilterType, metadata::Orientation,
};
use zaehler_app::{
    PortError,
    port::{ImageNormalizer, NormalizedJpeg, PhotoTime, PortResult},
};

/// Fehlercode für nicht unterstützte oder nicht dekodierbare Dateien
/// (Hinweis auf JPEG im Web-Adapter).
pub const UNSUPPORTED: &str = "photo.unsupported";
/// Fehlercode für Bilder über den Dekodier-Grenzen.
pub const TOO_LARGE: &str = "photo.too_large";

/// Grenzen gegen Dekompressionsbomben: Kantenlänge und Speicher.
const MAX_EDGE: u32 = 16_384;
const MAX_ALLOC: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOptions {
    /// Höchstzahl Pixel des Ergebnisses (`image.max_pixels`).
    pub max_pixels: u64,
    /// JPEG-Qualität 1 bis 100 (`image.jpeg_quality`).
    pub jpeg_quality: u8,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            max_pixels: 2_000_000,
            jpeg_quality: 85,
        }
    }
}

/// `ImageNormalizer` über das Crate `image`; die Arbeit läuft in
/// `spawn_blocking`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageCrateNormalizer {
    opts: ImageOptions,
}

impl ImageCrateNormalizer {
    #[must_use]
    pub fn new(opts: ImageOptions) -> Self {
        Self { opts }
    }
}

#[async_trait]
impl ImageNormalizer for ImageCrateNormalizer {
    // @impl SPEC_IMG_PROCESSING
    async fn normalize(&self, bytes: Bytes) -> PortResult<NormalizedJpeg> {
        let opts = self.opts;
        tokio::task::spawn_blocking(move || normalize(&bytes, opts))
            .await
            .map_err(|e| PortError::Backend(format!("Bildverarbeitung abgebrochen: {e}")))?
    }
}

#[allow(clippy::needless_pass_by_value)] // als Funktion für `map_err` verwendet
fn image_err(e: ImageError) -> PortError {
    match e {
        ImageError::Limits(_) => PortError::Invalid(TOO_LARGE),
        _ => PortError::Invalid(UNSUPPORTED),
    }
}

/// Synchrone Normalisierung.
// @impl SPEC_IMG_PROCESSING, SPEC_SEC_UPLOADS
pub fn normalize(bytes: &[u8], opts: ImageOptions) -> PortResult<NormalizedJpeg> {
    // Format nur über Magic Bytes; HEIC und alles andere wird abgelehnt.
    let format = image::guess_format(bytes).map_err(|_| PortError::Invalid(UNSUPPORTED))?;
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
    ) {
        return Err(PortError::Invalid(UNSUPPORTED));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(image_err)?;
    let taken_at = decoder.exif_metadata().ok().flatten().and_then(photo_time);
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(image_err)?;
    img.apply_orientation(orientation);

    let (w, h) = target_size(img.width(), img.height(), opts.max_pixels);
    if (w, h) != (img.width(), img.height()) {
        img = img.resize_exact(w, h, FilterType::Lanczos3);
    }
    // JPEG kennt keinen Alphakanal; der Encoder schreibt keine Metadaten.
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, opts.jpeg_quality.clamp(1, 100))
        .encode_image(&rgb)
        .map_err(|e| PortError::Backend(format!("JPEG-Kodierung: {e}")))?;
    Ok(NormalizedJpeg {
        bytes: Bytes::from(out),
        width: w,
        height: h,
        taken_at,
    })
}

/// Aufnahmezeit aus rohen EXIF-Daten (TIFF-Struktur): `DateTimeOriginal` mit
/// `OffsetTimeOriginal`, sonst `DateTime` mit `OffsetTime`. Fehlende oder
/// ungültige Angaben ergeben `None`.
// @impl SPEC_IMG_CAPTURE_TIME
fn photo_time(raw: Vec<u8>) -> Option<PhotoTime> {
    let data = exif::Reader::new().read_raw(raw).ok()?;
    let ascii = |tag| match data.get_field(tag, exif::In::PRIMARY).map(|f| &f.value) {
        Some(exif::Value::Ascii(v)) => v.first().cloned(),
        _ => None,
    };
    let (text, offset_tag) = ascii(exif::Tag::DateTimeOriginal)
        .map(|t| (t, exif::Tag::OffsetTimeOriginal))
        .or_else(|| ascii(exif::Tag::DateTime).map(|t| (t, exif::Tag::OffsetTime)))?;
    let mut dt = exif::DateTime::from_ascii(&text).ok()?;
    if let Some(offset) = ascii(offset_tag) {
        // ein ungültiger Offset lässt die lokale Zeit bestehen
        let _ = dt.parse_offset(&offset);
    }
    let local =
        NaiveDate::from_ymd_opt(i32::from(dt.year), u32::from(dt.month), u32::from(dt.day))?
            .and_hms_opt(
                u32::from(dt.hour),
                u32::from(dt.minute),
                u32::from(dt.second),
            )?;
    Some(PhotoTime {
        local,
        offset: dt
            .offset
            .and_then(|minutes| FixedOffset::east_opt(i32::from(minutes) * 60)),
    })
}

/// Größe mit höchstens `max_pixels` Pixeln bei gleichem Seitenverhältnis.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)] // Werte liegen zwischen 1 und MAX_EDGE
fn target_size(w: u32, h: u32, max_pixels: u64) -> (u32, u32) {
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= max_pixels || pixels == 0 {
        return (w, h);
    }
    let scale = (max_pixels as f64 / pixels as f64).sqrt();
    let nw = ((f64::from(w) * scale).floor() as u32).max(1);
    let nh = ((f64::from(h) * scale).floor() as u32).max(1);
    (nw, nh)
}

#[cfg(test)]
mod tests {
    use image::{ImageEncoder, Rgb, RgbImage, codecs::png::PngEncoder, codecs::webp::WebPEncoder};

    use super::*;

    /// Links rot, rechts blau.
    fn two_colors(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, _| {
            if x < w / 2 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        })
    }

    fn jpeg(img: &RgbImage) -> Vec<u8> {
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 95)
            .encode_image(img)
            .unwrap();
        out
    }

    /// APP1-Segment mit EXIF: Orientation und GPS-IFD (Breitengrad „N“).
    fn exif_app1(orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II*\0");
        tiff.extend_from_slice(&8u32.to_le_bytes());
        // IFD0 bei 8: 2 Einträge; GPS-IFD folgt bei 8 + 2 + 2*12 + 4 = 38
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0x8825u16.to_le_bytes()); // GPSInfo
        tiff.extend_from_slice(&4u16.to_le_bytes()); // LONG
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&38u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        // GPS-IFD: GPSLatitudeRef = "N"
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0001u16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes()); // ASCII
        tiff.extend_from_slice(&2u32.to_le_bytes());
        tiff.extend_from_slice(b"N\0\0\0");
        tiff.extend_from_slice(&0u32.to_le_bytes());

        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&tiff);
        let len = u16::try_from(payload.len() + 2).unwrap();
        let mut seg = vec![0xFF, 0xE1];
        seg.extend_from_slice(&len.to_be_bytes());
        seg.extend_from_slice(&payload);
        seg
    }

    fn with_exif(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let mut out = jpeg[..2].to_vec(); // SOI
        out.extend_from_slice(&exif_app1(orientation));
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    // @test SPEC_IMG_PROCESSING
    #[test]
    fn exif_orientation_is_applied_and_metadata_removed() {
        let input = with_exif(&jpeg(&two_colors(80, 40)), 6);
        assert!(contains(&input, b"Exif\0\0"));
        let out = normalize(&input, ImageOptions::default()).unwrap();
        // Ausrichtung 6: 90° im Uhrzeigersinn, linke Hälfte (rot) wird oben
        assert_eq!((out.width, out.height), (40, 80));
        let img = image::load_from_memory(&out.bytes).unwrap().to_rgb8();
        assert_eq!((img.width(), img.height()), (40, 80));
        let top = img.get_pixel(20, 10);
        let bottom = img.get_pixel(20, 70);
        assert!(top[0] > 200 && top[2] < 60, "oben rot: {top:?}");
        assert!(bottom[2] > 200 && bottom[0] < 60, "unten blau: {bottom:?}");
        // keine Metadaten: kein APP1-Segment, kein EXIF, kein GPS
        assert!(out.bytes.starts_with(&[0xFF, 0xD8]));
        assert!(!contains(&out.bytes, b"Exif"));
        assert!(!contains(&out.bytes, &[0xFF, 0xE1]));
        assert!(!contains(&out.bytes, b"N\0\0\0"));
    }

    /// EXIF mit Exif-IFD: `DateTimeOriginal` und optional `OffsetTimeOriginal`.
    fn exif_with_time(time: &str, offset: Option<&str>) -> Vec<u8> {
        let entries = 1 + u16::from(offset.is_some());
        // IFD0 bei 8 mit einem Eintrag; Exif-IFD bei 26; Daten dahinter
        let exif_ifd = 26u32;
        let data = exif_ifd + 2 + 12 * u32::from(entries) + 4;
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II*\0");
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x8769u16.to_le_bytes()); // ExifIFDPointer
        tiff.extend_from_slice(&4u16.to_le_bytes()); // LONG
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&exif_ifd.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let time = format!("{time}\0");
        tiff.extend_from_slice(&entries.to_le_bytes());
        tiff.extend_from_slice(&0x9003u16.to_le_bytes()); // DateTimeOriginal
        tiff.extend_from_slice(&2u16.to_le_bytes()); // ASCII
        tiff.extend_from_slice(&u32::try_from(time.len()).unwrap().to_le_bytes());
        tiff.extend_from_slice(&data.to_le_bytes());
        let offset = offset.map(|o| format!("{o}\0"));
        if let Some(o) = &offset {
            let at = data + u32::try_from(time.len()).unwrap();
            tiff.extend_from_slice(&0x9011u16.to_le_bytes()); // OffsetTimeOriginal
            tiff.extend_from_slice(&2u16.to_le_bytes());
            tiff.extend_from_slice(&u32::try_from(o.len()).unwrap().to_le_bytes());
            tiff.extend_from_slice(&at.to_le_bytes());
        }
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(time.as_bytes());
        if let Some(o) = &offset {
            tiff.extend_from_slice(o.as_bytes());
        }
        tiff
    }

    fn with_tiff(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(tiff);
        let len = u16::try_from(payload.len() + 2).unwrap();
        let mut out = jpeg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    // @test SPEC_IMG_CAPTURE_TIME, SPEC_IMG_PROCESSING
    #[test]
    fn capture_time_is_read_before_metadata_is_removed() {
        let base = jpeg(&two_colors(8, 8));
        let at = NaiveDate::from_ymd_opt(2026, 1, 10)
            .unwrap()
            .and_hms_opt(8, 15, 30)
            .unwrap();

        let input = with_tiff(
            &base,
            &exif_with_time("2026:01:10 08:15:30", Some("+01:00")),
        );
        let out = normalize(&input, ImageOptions::default()).unwrap();
        assert_eq!(
            out.taken_at,
            Some(PhotoTime {
                local: at,
                offset: FixedOffset::east_opt(3600),
            })
        );
        assert!(!contains(&out.bytes, b"2026:01:10"), "Metadaten entfernt");

        let input = with_tiff(&base, &exif_with_time("2026:01:10 08:15:30", None));
        let out = normalize(&input, ImageOptions::default()).unwrap();
        assert_eq!(out.taken_at.map(|t| (t.local, t.offset)), Some((at, None)));

        for bad in ["0000:00:00 00:00:00", "    :  :     :  :  "] {
            let input = with_tiff(&base, &exif_with_time(bad, None));
            let out = normalize(&input, ImageOptions::default()).unwrap();
            assert_eq!(out.taken_at, None, "{bad}");
        }
        // nur Ausrichtung, keine Zeit
        let out = normalize(&with_exif(&base, 1), ImageOptions::default()).unwrap();
        assert_eq!(out.taken_at, None);
    }

    // @test SPEC_IMG_PROCESSING
    #[test]
    fn large_png_is_downscaled_keeping_aspect_ratio() {
        let mut png = Vec::new();
        let img = two_colors(3000, 1000);
        PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 3000, 1000, image::ExtendedColorType::Rgb8)
            .unwrap();
        let out = normalize(&png, ImageOptions::default()).unwrap();
        assert!(u64::from(out.width) * u64::from(out.height) <= 2_000_000);
        assert!(
            out.width >= 2440 && out.height >= 813,
            "{}x{}",
            out.width,
            out.height
        );
        let ratio = f64::from(out.width) / f64::from(out.height);
        assert!((ratio - 3.0).abs() < 0.01);
        assert_eq!(image::guess_format(&out.bytes).unwrap(), ImageFormat::Jpeg);
    }

    // @test SPEC_IMG_PROCESSING
    #[test]
    fn webp_is_accepted() {
        let mut webp = Vec::new();
        let img = two_colors(10, 10);
        WebPEncoder::new_lossless(&mut webp)
            .write_image(img.as_raw(), 10, 10, image::ExtendedColorType::Rgb8)
            .unwrap();
        let out = normalize(&webp, ImageOptions::default()).unwrap();
        assert_eq!((out.width, out.height), (10, 10));
    }

    // @test SPEC_IMG_PROCESSING, SPEC_SEC_UPLOADS
    #[test]
    fn heic_text_and_broken_files_are_rejected() {
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic\0\0\0\0".to_vec();
        let text = b"Zaehlerstand;12345\n".to_vec();
        let truncated = jpeg(&two_colors(8, 8))[..20].to_vec();
        let gif = b"GIF89a\x01\0\x01\0\0\0\0;".to_vec();
        for input in [heic, text, truncated, gif, Vec::new()] {
            assert!(
                matches!(
                    normalize(&input, ImageOptions::default()),
                    Err(PortError::Invalid(UNSUPPORTED))
                ),
                "{:?}",
                &input[..input.len().min(12)]
            );
        }
    }

    // @test SPEC_SEC_UPLOADS
    #[test]
    fn oversized_dimensions_are_rejected_before_decoding() {
        // PNG-Header mit 20000 × 20000 Pixeln, ohne Bilddaten
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&[0; 3], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        // IHDR: Breite und Höhe ab Byte 16; CRC passt danach nicht mehr,
        // daher entweder Grenzfehler oder Dekodierfehler, nie ein Ergebnis
        png[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&20_000u32.to_be_bytes());
        assert!(matches!(
            normalize(&png, ImageOptions::default()),
            Err(PortError::Invalid(_))
        ));
    }

    #[test]
    fn target_size_keeps_small_images() {
        assert_eq!(target_size(100, 50, 2_000_000), (100, 50));
        assert_eq!(target_size(4000, 3000, 2_000_000), (1632, 1224));
    }

    // @test SPEC_IMG_PROCESSING
    #[tokio::test]
    async fn adapter_runs_in_blocking_pool() {
        let out = ImageCrateNormalizer::default()
            .normalize(Bytes::from(jpeg(&two_colors(4, 4))))
            .await
            .unwrap();
        assert_eq!((out.width, out.height), (4, 4));
    }
}
