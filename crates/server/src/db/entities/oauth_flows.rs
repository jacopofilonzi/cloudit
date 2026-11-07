//! `oauth_flows`: one started authorization at a provider (single use, 10 min). The
//! `state` and the browser-binding cookie are stored as SHA-256 hashes; the PKCE
//! verifier and the nonce are kept as they are (the server sends them later) in
//! [`Hidden`] columns, which print `***` in `Debug`.

use std::fmt;

use sea_orm::entity::prelude::*;

/// A text column whose value must not appear in logs.
#[derive(Clone, PartialEq, Eq, DeriveValueType)]
pub struct Hidden(pub String);

impl fmt::Debug for Hidden {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oauth_flows")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub state_hash: String,
    pub browser_binding_hash: String,
    pub provider_id: String,
    /// `login`, `link` or `onboarding_link`.
    pub purpose: String,
    /// The signed-in user (`link`).
    pub user_id: Option<Uuid>,
    /// The pending signup to attach (`onboarding_link`).
    pub pending_signup_id: Option<Uuid>,
    pub pkce_verifier: Hidden,
    pub nonce: Hidden,
    /// Relative path to send the browser to afterwards.
    pub return_to: String,
    pub invite_token_hash: Option<String>,
    pub created_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub used_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
