//! `identities`: an account's link to a provider subject. `(provider_id, subject)` is
//! unique: one provider account belongs to at most one cloudit account. `email` is the
//! provider's last known email (informational; the account's email is `users.email`).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "identities")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider_id: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub created_at: DateTimeUtc,
    pub last_login_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
