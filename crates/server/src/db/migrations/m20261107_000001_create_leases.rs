//! `leases`: named, expiring locks held by one server instance (see `db::lease`).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Leases {
    Table,
    Name,
    Holder,
    ExpiresAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Leases::Table)
                    .col(ColumnDef::new(Leases::Name).text().not_null().primary_key())
                    .col(ColumnDef::new(Leases::Holder).text().not_null())
                    .col(
                        ColumnDef::new(Leases::ExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}
