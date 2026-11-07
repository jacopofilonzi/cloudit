//! `oauth_flows`: one started authorization (see the module docs of `oauth`).
//!
//! [`create`] stores SHA-256 of `state` and of the browser-binding cookie, with the
//! PKCE verifier and the nonce. [`consume`] is one conditional UPDATE, `used_at = now
//! WHERE state_hash = ? AND provider_id = ? AND browser_binding_hash = ? AND used_at IS
//! NULL AND expires_at > now`: a flow completes at most once, only before it expires,
//! only for its provider and only in the browser that started it.

use std::time::Duration;

use anyhow::Context as _;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
};
use uuid::Uuid;

use crate::auth::tokens;
use crate::config::Secret;
use crate::db::entities::oauth_flows::{self, Hidden};

/// Validity of a started flow.
pub const TTL: Duration = Duration::from_secs(10 * 60);

/// Why the flow was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Login,
    Link,
    /// Prove an existing account with another provider to attach a pending signup.
    OnboardingLink,
}

impl Purpose {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Link => "link",
            Self::OnboardingLink => "onboarding_link",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Login, Self::Link, Self::OnboardingLink]
            .into_iter()
            .find(|p| p.as_str() == text)
    }
}

/// What a new flow remembers.
pub struct NewFlow<'a> {
    pub provider_id: &'a str,
    pub purpose: Purpose,
    pub user_id: Option<Uuid>,
    pub pending_signup_id: Option<Uuid>,
    pub return_to: String,
    pub invite_token_hash: Option<String>,
}

/// The secrets of a started flow, sent to the provider.
pub struct Started {
    pub state: Secret,
    pub verifier: Secret,
    pub nonce: Secret,
}

/// Stores a new flow bound to the browser cookie value `binding`.
pub async fn create(
    db: &impl ConnectionTrait,
    new: NewFlow<'_>,
    binding: &Secret,
) -> anyhow::Result<Started> {
    let started = Started {
        state: tokens::new_token()?,
        verifier: tokens::new_token()?,
        nonce: tokens::new_token()?,
    };
    let now = crate::time::now();
    let model = oauth_flows::ActiveModel {
        id: Set(Uuid::now_v7()),
        state_hash: Set(tokens::hash_token(started.state.expose())),
        browser_binding_hash: Set(tokens::hash_token(binding.expose())),
        provider_id: Set(new.provider_id.to_owned()),
        purpose: Set(new.purpose.as_str().to_owned()),
        user_id: Set(new.user_id),
        pending_signup_id: Set(new.pending_signup_id),
        pkce_verifier: Set(Hidden(started.verifier.expose().to_owned())),
        nonce: Set(Hidden(started.nonce.expose().to_owned())),
        return_to: Set(new.return_to),
        invite_token_hash: Set(new.invite_token_hash),
        created_at: Set(now),
        expires_at: Set(crate::time::add(now, TTL)?),
        used_at: Set(None),
    };
    oauth_flows::Entity::insert(model)
        .exec_without_returning(db)
        .await
        .context("storing a sign-in flow")?;
    Ok(started)
}

/// Consumes the flow (see the module docs); `None` if no usable flow matches.
pub async fn consume(
    db: &impl ConnectionTrait,
    provider_id: &str,
    state: &str,
    binding: &str,
) -> anyhow::Result<Option<oauth_flows::Model>> {
    let state_hash = tokens::hash_token(state);
    let now = crate::time::now();
    let result = oauth_flows::Entity::update_many()
        .col_expr(oauth_flows::Column::UsedAt, Expr::value(now))
        .filter(oauth_flows::Column::StateHash.eq(state_hash.clone()))
        .filter(oauth_flows::Column::ProviderId.eq(provider_id))
        .filter(oauth_flows::Column::BrowserBindingHash.eq(tokens::hash_token(binding)))
        .filter(oauth_flows::Column::UsedAt.is_null())
        .filter(oauth_flows::Column::ExpiresAt.gt(now))
        .exec(db)
        .await
        .context("consuming a sign-in flow")?;
    if result.rows_affected != 1 {
        return Ok(None);
    }
    oauth_flows::Entity::find()
        .filter(oauth_flows::Column::StateHash.eq(state_hash))
        .one(db)
        .await
        .context("loading a sign-in flow")
}
