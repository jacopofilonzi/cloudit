//! `auth_providers`: identity providers configured by admins (see `idp`). The id is a
//! durable slug; `client_secret_enc` is `crypto::SecretBox` output bound to the id;
//! `admin_groups` / `allowed_groups` are JSON arrays of strings; deleting is soft.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_providers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub kind: String,
    pub display_name: String,
    pub icon: String,
    pub issuer_url: Option<String>,
    pub tenant: Option<String>,
    pub client_id: String,
    pub client_secret_enc: Vec<u8>,
    pub scopes: String,
    pub groups_claim: Option<String>,
    pub admin_groups: String,
    pub allowed_groups: String,
    pub enabled: bool,
    pub position: i32,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub deleted_at: Option<DateTimeUtc>,
}

impl ActiveModelBehavior for ActiveModel {}
