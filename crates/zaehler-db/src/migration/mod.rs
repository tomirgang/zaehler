//! Schema-Migrationen. Rust-basiert (sea-orm-migration), damit eine Quelle
//! für PostgreSQL und SQLite genügt.

use sea_orm_migration::prelude::*;

mod m20261015_000001_core;
mod m20261015_000002_mvp;
mod m20261015_000003_ai_drafts;
mod m20261016_000004_draft_taken_at;
mod m20261016_000005_draft_analysis_started_at;
mod m20261016_000006_members;
mod m20261016_000007_reading_revisions;
mod m20261016_000008_meter_v1;
mod m20261016_000009_jobs;
mod m20261016_000010_expectation;
mod m20261017_000011_tariffs;
mod m20261017_000012_documents;
mod m20261018_000013_vehicles;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261015_000001_core::Migration),
            Box::new(m20261015_000002_mvp::Migration),
            Box::new(m20261015_000003_ai_drafts::Migration),
            Box::new(m20261016_000004_draft_taken_at::Migration),
            Box::new(m20261016_000005_draft_analysis_started_at::Migration),
            Box::new(m20261016_000006_members::Migration),
            Box::new(m20261016_000007_reading_revisions::Migration),
            Box::new(m20261016_000008_meter_v1::Migration),
            Box::new(m20261016_000009_jobs::Migration),
            Box::new(m20261016_000010_expectation::Migration),
            Box::new(m20261017_000011_tariffs::Migration),
            Box::new(m20261017_000012_documents::Migration),
            Box::new(m20261018_000013_vehicles::Migration),
        ]
    }
}
