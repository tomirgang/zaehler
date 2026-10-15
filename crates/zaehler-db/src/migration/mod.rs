//! Schema-Migrationen. Rust-basiert (sea-orm-migration), damit eine Quelle
//! für PostgreSQL und SQLite genügt.

use sea_orm_migration::prelude::*;

mod m20261015_000001_core;
mod m20261015_000002_mvp;
mod m20261015_000003_ai_drafts;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261015_000001_core::Migration),
            Box::new(m20261015_000002_mvp::Migration),
            Box::new(m20261015_000003_ai_drafts::Migration),
        ]
    }
}
