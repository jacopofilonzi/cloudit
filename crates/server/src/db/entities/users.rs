//! `users`: one account. `email` is stored trimmed and lowercased (unique).
//! `password_hash` is null for accounts that only sign in through an identity provider
//! (milestone 3c). Effective admin = `admin || admin_from_groups`. An account signs in
//! only while enabled and approved ([`Model::is_active`]).

use cloudit_core::protocol::ApprovalStatus;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub email: String,
    pub email_verified: bool,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub admin: bool,
    pub admin_from_groups: bool,
    pub disabled_at: Option<DateTimeUtc>,
    /// `approved`, `pending` or `rejected` (`ApprovalStatus`).
    pub approval_status: String,
    pub approval_decided_at: Option<DateTimeUtc>,
    /// The admin who approved or rejected (no foreign key, see migration 000006).
    pub approval_decided_by: Option<Uuid>,
    /// `password`, `invite` or a provider id; null for the first admin and older
    /// accounts.
    pub signup_source: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// Manual or group-derived admin.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.admin || self.admin_from_groups
    }

    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled_at.is_some()
    }

    /// The approval state; an unknown stored value counts as `pending` (fails closed).
    #[must_use]
    pub fn approval(&self) -> ApprovalStatus {
        ApprovalStatus::parse(&self.approval_status).unwrap_or(ApprovalStatus::Pending)
    }

    /// Can sign in and use its credentials: enabled and approved.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.is_disabled() && self.approval() == ApprovalStatus::Approved
    }
}
