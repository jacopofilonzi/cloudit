//! `folders.emoji`: an optional emoji the user picks to tell folders apart in the UIs
//! (validated by `validate::emoji`). NULL on existing rows (= no emoji).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Folders {
    Table,
    Emoji,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Folders::Table)
                    .add_column(ColumnDef::new(Folders::Emoji).text().null())
                    .to_owned(),
            )
            .await
    }

    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }
}
