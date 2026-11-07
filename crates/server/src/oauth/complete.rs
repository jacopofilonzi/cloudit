//! What a verified provider callback does, by flow purpose.
//!
//! First, a provider with `allowed_groups` refuses identities in none of them
//! (`group_not_allowed`). Then:
//!
//! - **login, known identity**: the account must be enabled and approved
//!   (`account_disabled`, `account_rejected`, `account_pending`); the
//!   identity's email and last sign-in are updated; for OIDC providers
//!   `admin_from_groups` is recomputed ([`apply_groups`]); a session is created.
//! - **login, unknown identity**: with the `autolink_email` setting on, an email the
//!   provider marks verified that belongs to exactly one account whose own email is
//!   verified links the identity to that account and signs in. Otherwise a pending
//!   signup is stored and the browser goes to `/onboarding`. Unverified emails are
//!   never trusted for linking.
//! - **link** (from a signed-in session): the session must still be the flow's user;
//!   an identity of another account is `identity_in_use`, one of this account a no-op.
//!   Linking does not change `admin_from_groups` (it is not a sign-in).
//! - **onboarding_link**: the user proved an existing account with an identity that
//!   is already linked; the pending identity is attached to that account (consuming
//!   the pending signup first), then the user is signed in.
//!
//! Every path that signs in or attaches to an existing account requires it to be
//! enabled and approved. A rejected account keeps its identities, so signing in with
//! one answers `account_rejected` and never creates a new account. New accounts
//! (onboarding) are approved or pending per the provider's `new_accounts`.
//!
//! `admin_from_groups` reflects the last sign-in through an OIDC provider: true iff
//! the provider has `admin_groups` and the identity is in one of them. Sign-ins
//! through GitHub or Discord leave it unchanged.

use anyhow::Context as _;
use cloudit_core::protocol::ProviderProtocol;
use sea_orm::TransactionTrait as _;
use uuid::Uuid;

use super::flows::Purpose;
use super::{identities, pending};
use crate::accounts::{admin, settings, users};
use crate::config::Secret;
use crate::db::entities::{identities as identity_entity, oauth_flows, users as user_entity};
use crate::errors::ApiError;
use crate::idp::claims::ExternalIdentity;
use crate::idp::store::Provider;
use crate::state::AppState;

/// Result of a successful callback.
pub enum Outcome {
    /// Create a session for this user (and drop the onboarding cookie if set).
    SignedIn {
        user_id: Uuid,
        clear_onboarding: bool,
    },
    /// The identity was linked to the signed-in account.
    Linked,
    /// Unknown identity: set the onboarding cookie and go to onboarding.
    Onboarding { token: Secret },
}

/// `true` if `identity` gets admin from `provider`'s groups (OIDC providers only).
#[must_use]
pub fn admin_by_groups(provider: &Provider, identity: &ExternalIdentity) -> bool {
    provider.protocol() == ProviderProtocol::Oidc
        && identity.admin_from_groups(&provider.fields.admin_groups)
}

/// Recomputes `admin_from_groups` after a sign-in through `provider` (OIDC only).
pub async fn apply_groups(
    state: &AppState,
    provider: &Provider,
    user_id: Uuid,
    identity: &ExternalIdentity,
) -> anyhow::Result<()> {
    if provider.protocol() != ProviderProtocol::Oidc {
        return Ok(());
    }
    let value = identity.admin_from_groups(&provider.fields.admin_groups);
    admin::set_admin_from_groups(&state.db, user_id, value).await
}

async fn active_user(state: &AppState, id: Uuid) -> Result<user_entity::Model, ApiError> {
    let user = users::find(&state.db, id)
        .await?
        .context("identity of a missing user")?;
    users::ensure_active(&user)?;
    Ok(user)
}

async fn sign_in_existing(
    state: &AppState,
    provider: &Provider,
    link: &identity_entity::Model,
    identity: &ExternalIdentity,
) -> Result<Outcome, ApiError> {
    let user = active_user(state, link.user_id).await?;
    identities::record_sign_in(&state.db, link.id, identity).await?;
    apply_groups(state, provider, user.id, identity).await?;
    Ok(Outcome::SignedIn {
        user_id: user.id,
        clear_onboarding: false,
    })
}

async fn login_unknown(
    state: &AppState,
    provider: &Provider,
    flow: &oauth_flows::Model,
    identity: &ExternalIdentity,
) -> Result<Outcome, ApiError> {
    if settings::autolink_email(&state.db).await?
        && identity.email_verified
        && let Some(email) = &identity.email
        && let Some(user) = users::find_by_email(&state.db, email).await?
        && user.email_verified
    {
        users::ensure_active(&user)?;
        identities::insert(&state.db, user.id, &provider.id, identity, true).await?;
        apply_groups(state, provider, user.id, identity).await?;
        tracing::info!(user_id = %user.id, provider = %provider.id, "identity auto-linked by verified email");
        return Ok(Outcome::SignedIn {
            user_id: user.id,
            clear_onboarding: false,
        });
    }
    let token = pending::create(
        &state.db,
        &provider.id,
        identity,
        flow.invite_token_hash.clone(),
    )
    .await?;
    Ok(Outcome::Onboarding { token })
}

/// Completes a callback whose flow was consumed and whose identity was verified.
/// `session_user` is the user of the request's web session, if any.
pub async fn finish(
    state: &AppState,
    provider: &Provider,
    flow: &oauth_flows::Model,
    identity: &ExternalIdentity,
    session_user: Option<Uuid>,
) -> Result<Outcome, ApiError> {
    if !identity.allowed(&provider.fields.allowed_groups) {
        return Err(ApiError::GroupNotAllowed);
    }
    let purpose = Purpose::parse(&flow.purpose).context("unknown flow purpose")?;
    let existing = identities::find(&state.db, &provider.id, &identity.subject).await?;
    match purpose {
        Purpose::Login => match existing {
            Some(link) => sign_in_existing(state, provider, &link, identity).await,
            None => login_unknown(state, provider, flow, identity).await,
        },
        Purpose::Link => {
            let user_id = flow.user_id.context("link flow without a user")?;
            if session_user != Some(user_id) {
                return Err(ApiError::OauthStateInvalid);
            }
            let user = active_user(state, user_id).await?;
            match existing {
                Some(link) if link.user_id == user.id => {}
                Some(_) => return Err(ApiError::IdentityInUse),
                None => {
                    identities::insert(&state.db, user.id, &provider.id, identity, false).await?;
                    tracing::info!(user_id = %user.id, provider = %provider.id, "identity linked");
                }
            }
            Ok(Outcome::Linked)
        }
        Purpose::OnboardingLink => {
            let pending_id = flow
                .pending_signup_id
                .context("onboarding flow without a pending signup")?;
            let pending = pending::find_usable_by_id(&state.db, pending_id)
                .await?
                .ok_or(ApiError::OnboardingExpired)?;
            let link = existing.ok_or(ApiError::IdentityNotLinked)?;
            let user = active_user(state, link.user_id).await?;
            let pending_identity = pending::identity(&pending)?;
            let tx = state.db.begin().await.map_err(anyhow::Error::from)?;
            if !pending::consume(&tx, pending.id).await? {
                return Err(ApiError::OnboardingExpired);
            }
            identities::insert(&tx, user.id, &pending.provider_id, &pending_identity, true).await?;
            tx.commit().await.map_err(anyhow::Error::from)?;
            identities::record_sign_in(&state.db, link.id, identity).await?;
            apply_groups(state, provider, user.id, identity).await?;
            tracing::info!(
                user_id = %user.id,
                provider = %pending.provider_id,
                "pending identity linked after signing in with another provider"
            );
            Ok(Outcome::SignedIn {
                user_id: user.id,
                clear_onboarding: true,
            })
        }
    }
}
