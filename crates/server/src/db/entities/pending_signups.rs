//! `pending_signups`: an identity that authenticated at a provider but belongs to no
//! account yet, waiting for onboarding (single use, 15 min). Only the SHA-256 of its
//! token (the `cloudit_onboarding` cookie) is stored. `groups` is a JSON array.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "pending_signups")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub token_hash: String,
    pub provider_id: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: String,
    pub groups: String,
    /// SHA-256 of an invite token carried from the sign-in start, if any.
    pub invite_token_hash: Option<String>,
    pub created_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub used_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
