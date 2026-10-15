//! Driven-Adapter für den Port `BlobStore` (ADR_006, SPEC_STORE_BLOBS) über
//! `object_store`: lokales Verzeichnis oder S3-kompatibler Objektspeicher.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt, stream};
use object_store::{
    ObjectStore, ObjectStoreExt, PutPayload, aws::AmazonS3Builder, local::LocalFileSystem,
    path::Path,
};
use zaehler_app::{
    PortError,
    ids::{BlobKey, BlobKind, HouseholdId},
    port::{BlobStore, PortResult},
};

/// Zugang zu einem S3-kompatiblen Objektspeicher. Nicht gesetzte Werte
/// kommen aus den Umgebungsvariablen `AWS_*`.
#[derive(Debug, Clone, Default)]
pub struct S3Options {
    pub bucket: String,
    /// z.B. `http://minio:9000`; ohne Angabe AWS.
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    /// Unverschlüsselte Verbindung zum Endpunkt zulassen.
    pub allow_http: bool,
}

/// `BlobStore` auf einem beliebigen `ObjectStore`.
#[derive(Debug, Clone)]
pub struct ObjectStoreBlobStore {
    store: Arc<dyn ObjectStore>,
}

#[allow(clippy::needless_pass_by_value)] // als Funktion für `map_err` verwendet
fn store_err(e: object_store::Error) -> PortError {
    match e {
        object_store::Error::NotFound { .. } => PortError::NotFound,
        other => PortError::Backend(other.to_string()),
    }
}

/// Objekt fehlt. Manche S3-Implementierungen melden fehlende Schlüssel in
/// `DeleteObjects` als Fehler je Schlüssel (`NoSuchKey`), den `object_store`
/// als `Generic` weitergibt; AWS und MinIO melden Erfolg.
fn is_missing(e: &object_store::Error) -> bool {
    match e {
        object_store::Error::NotFound { .. } => true,
        object_store::Error::Generic { source, .. } => source.to_string().contains("NoSuchKey"),
        _ => false,
    }
}

fn path(key: &BlobKey) -> Path {
    Path::from(key.as_str())
}

impl ObjectStoreBlobStore {
    #[must_use]
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// Lokales Verzeichnis; wird bei Bedarf angelegt. Leere Verzeichnisse
    /// werden beim Löschen entfernt.
    pub fn local(dir: &std::path::Path) -> PortResult<Self> {
        std::fs::create_dir_all(dir).map_err(|e| {
            PortError::Backend(format!("Verzeichnis {} nicht anlegbar: {e}", dir.display()))
        })?;
        let fs = LocalFileSystem::new_with_prefix(dir)
            .map_err(store_err)?
            .with_automatic_cleanup(true);
        Ok(Self::new(Arc::new(fs)))
    }

    /// S3-kompatibler Objektspeicher (Pfad-Adressierung, passend für MinIO,
    /// Garage, Ceph RGW).
    pub fn s3(o: &S3Options) -> PortResult<Self> {
        let mut b = AmazonS3Builder::from_env()
            .with_bucket_name(&o.bucket)
            .with_allow_http(o.allow_http);
        if let Some(v) = &o.endpoint {
            b = b.with_endpoint(v);
        }
        if let Some(v) = &o.region {
            b = b.with_region(v);
        }
        if let Some(v) = &o.access_key_id {
            b = b.with_access_key_id(v);
        }
        if let Some(v) = &o.secret_access_key {
            b = b.with_secret_access_key(v);
        }
        Ok(Self::new(Arc::new(b.build().map_err(store_err)?)))
    }

    /// Löscht die Objekte; bereits fehlende sind kein Fehler.
    async fn delete_all(&self, paths: Vec<Path>) -> PortResult<()> {
        let results: Vec<_> = self
            .store
            .delete_stream(stream::iter(paths.into_iter().map(Ok)).boxed())
            .collect()
            .await;
        for r in results {
            match r {
                Err(e) if !is_missing(&e) => return Err(store_err(e)),
                _ => {}
            }
        }
        Ok(())
    }
}

#[async_trait]
impl BlobStore for ObjectStoreBlobStore {
    // @impl SPEC_STORE_BLOBS
    async fn put(&self, key: &BlobKey, bytes: Bytes) -> PortResult<()> {
        self.store
            .put(&path(key), PutPayload::from_bytes(bytes))
            .await
            .map(|_| ())
            .map_err(store_err)
    }

    async fn get(&self, key: &BlobKey) -> PortResult<Bytes> {
        self.store
            .get(&path(key))
            .await
            .map_err(store_err)?
            .bytes()
            .await
            .map_err(store_err)
    }

    async fn copy(&self, from: &BlobKey, to: &BlobKey) -> PortResult<()> {
        self.store
            .copy(&path(from), &path(to))
            .await
            .map_err(store_err)
    }

    async fn delete(&self, key: &BlobKey) -> PortResult<()> {
        self.delete_all(vec![path(key)]).await
    }

    async fn delete_household(&self, household: HouseholdId) -> PortResult<()> {
        let prefix = Path::from(BlobKey::household_prefix(household).as_str());
        let paths: Vec<Path> = self
            .store
            .list(Some(&prefix))
            .map_ok(|m| m.location)
            .try_collect()
            .await
            .map_err(store_err)?;
        self.delete_all(paths).await
    }

    // @impl SPEC_STORE_BLOBS, SPEC_OPS_HOUSEKEEPING
    async fn list_tmp_older_than(&self, cutoff: DateTime<Utc>) -> PortResult<Vec<BlobKey>> {
        // je Haushalt nur das Verzeichnis tmp/ listen, nicht alle Fotos
        let households = match self
            .store
            .list_with_delimiter(Some(&Path::from("households")))
            .await
        {
            Ok(list) => list.common_prefixes,
            Err(object_store::Error::NotFound { .. }) => Vec::new(),
            Err(e) => return Err(store_err(e)),
        };
        let mut keys = Vec::new();
        for household in households {
            let tmp = household.clone().join("tmp");
            // Objekte, die ein paralleler Lauf während des Listings löscht,
            // werden übersprungen.
            let objects: Vec<_> = self
                .store
                .list(Some(&tmp))
                .filter(|r| {
                    std::future::ready(!matches!(r, Err(object_store::Error::NotFound { .. })))
                })
                .try_collect()
                .await
                .map_err(store_err)?;
            keys.extend(
                objects
                    .into_iter()
                    .filter(|m| m.last_modified < cutoff)
                    .filter_map(|m| BlobKey::parse(m.location.as_ref()))
                    .filter(|k| k.kind() == BlobKind::Tmp),
            );
        }
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use object_store::memory::InMemory;
    use zaehler_app::{
        port::SystemClock,
        testing::{InMemoryStore, suite},
        usecase::housekeeping::Housekeeping,
    };

    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("zaehler-storage-{}", uuid::Uuid::now_v7()))
    }

    // @test SPEC_STORE_BLOBS
    #[tokio::test]
    async fn object_store_in_memory_passes_blob_suite() {
        suite::blob_store(&ObjectStoreBlobStore::new(Arc::new(InMemory::new()))).await;
    }

    // @test SPEC_STORE_BLOBS
    #[tokio::test]
    async fn local_filesystem_passes_blob_suite() {
        let dir = temp_dir();
        suite::blob_store(&ObjectStoreBlobStore::local(&dir).unwrap()).await;
        std::fs::remove_dir_all(dir).ok();
    }

    /// Läuft nur, wenn `ZAEHLER_TEST_S3_ENDPOINT` gesetzt ist (CI: MinIO).
    /// Weitere Variablen: `ZAEHLER_TEST_S3_BUCKET`, `ZAEHLER_TEST_S3_ACCESS_KEY`,
    /// `ZAEHLER_TEST_S3_SECRET_KEY`.
    // @test SPEC_STORE_BLOBS
    #[tokio::test]
    async fn s3_passes_blob_suite() {
        let Ok(endpoint) = std::env::var("ZAEHLER_TEST_S3_ENDPOINT") else {
            eprintln!("ZAEHLER_TEST_S3_ENDPOINT nicht gesetzt, Test übersprungen");
            return;
        };
        let env = |k: &str| std::env::var(k).ok();
        let store = ObjectStoreBlobStore::s3(&S3Options {
            bucket: env("ZAEHLER_TEST_S3_BUCKET").unwrap_or_else(|| "zaehler-test".into()),
            allow_http: endpoint.starts_with("http://"),
            endpoint: Some(endpoint),
            region: Some("us-east-1".into()),
            access_key_id: env("ZAEHLER_TEST_S3_ACCESS_KEY"),
            secret_access_key: env("ZAEHLER_TEST_S3_SECRET_KEY"),
        })
        .unwrap();
        suite::blob_store(&store).await;
    }

    // @test SPEC_OPS_HOUSEKEEPING
    #[tokio::test]
    async fn housekeeping_on_filesystem_is_idempotent_in_parallel() {
        let dir = temp_dir();
        let blobs = Arc::new(ObjectStoreBlobStore::local(&dir).unwrap());
        let h = HouseholdId(uuid::Uuid::now_v7());
        let old: Vec<BlobKey> = (0..20)
            .map(|_| BlobKey::tmp(h, uuid::Uuid::now_v7()))
            .collect();
        let fresh = BlobKey::tmp(h, uuid::Uuid::now_v7());
        for key in old.iter().chain([&fresh]) {
            blobs.put(key, Bytes::from_static(b"a;b")).await.unwrap();
        }
        let day_ago = SystemTime::now() - Duration::from_secs(25 * 3600);
        for key in &old {
            std::fs::File::options()
                .write(true)
                .open(dir.join(key.as_str()))
                .unwrap()
                .set_modified(day_ago)
                .unwrap();
        }
        let hk = Housekeeping::new(
            Arc::new(InMemoryStore::default()),
            blobs.clone(),
            Arc::new(SystemClock),
            chrono::Duration::hours(24),
        );
        let (a, b) = tokio::join!(hk.execute(), hk.execute());
        a.unwrap();
        b.unwrap();
        assert_eq!(
            blobs.list_tmp_older_than(Utc::now()).await.unwrap(),
            vec![fresh]
        );
        assert_eq!(hk.execute().await.unwrap().tmp, 0);
        std::fs::remove_dir_all(dir).ok();
    }
}
