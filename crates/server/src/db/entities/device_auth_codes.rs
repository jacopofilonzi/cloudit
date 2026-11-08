//! `device_auth_codes`: single-use authorization codes (2 min) of the loopback + PKCE
//! device login.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "device_auth_codes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub code_hash: String,
    pub user_id: Uuid,
    /// base64url SHA-256 of the client's verifier (S256).
    pub code_challenge: String,
    pub redirect_uri: String,
    pub device_name: String,
    pub os: String,
    /// `None` = a token that never expires.
    pub token_ttl_secs: Option<i64>,
    pub created_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub used_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
