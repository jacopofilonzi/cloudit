//! `identities`: links between accounts and provider subjects. `(provider_id,
//! subject)` is unique, so inserting a second link for the same provider account fails
//! with `identity_in_use` whatever the race.

use std::collections::HashMap;

use anyhow::Context as _;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
    QueryOrder as _,
};
use uuid::Uuid;

use crate::db::entities::identities;
use crate::errors::ApiError;
use crate::idp::claims::ExternalIdentity;

/// The identity of `subject` at `provider_id`, if linked.
pub async fn find(
    db: &impl ConnectionTrait,
    provider_id: &str,
    subject: &str,
) -> anyhow::Result<Option<identities::Model>> {
    identities::Entity::find()
        .filter(identities::Column::ProviderId.eq(provider_id))
        .filter(identities::Column::Subject.eq(subject))
        .one(db)
        .await
        .context("loading an identity")
}

/// Links `identity` at `provider_id` to `user_id` (`signed_in`: also records a sign-in
/// now); `identity_in_use` if that provider account is linked already.
pub async fn insert(
    db: &impl ConnectionTrait,
    user_id: Uuid,
    provider_id: &str,
    identity: &ExternalIdentity,
    signed_in: bool,
) -> Result<identities::Model, ApiError> {
    let now = crate::time::now();
    let model = identities::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(user_id),
        provider_id: Set(provider_id.to_owned()),
        subject: Set(identity.subject.clone()),
        email: Set(identity.email.clone()),
        email_verified: Set(identity.email_verified),
        created_at: Set(now),
        last_login_at: Set(signed_in.then_some(now)),
    };
    match identities::Entity::insert(model)
        .exec_with_returning(db)
        .await
    {
        Ok(row) => Ok(row),
        Err(err) if crate::accounts::users::is_unique_violation(&err) => {
            Err(ApiError::IdentityInUse)
        }
        Err(err) => Err(anyhow::Error::new(err)
            .context("linking an identity")
            .into()),
    }
}

/// Records a sign-in: the provider's current email and the time.
pub async fn record_sign_in(
    db: &impl ConnectionTrait,
    id: Uuid,
    identity: &ExternalIdentity,
) -> anyhow::Result<()> {
    identities::Entity::update_many()
        .col_expr(
            identities::Column::Email,
            Expr::value(identity.email.clone()),
        )
        .col_expr(
            identities::Column::EmailVerified,
            Expr::value(identity.email_verified),
        )
        .col_expr(
            identities::Column::LastLoginAt,
            Expr::value(crate::time::now()),
        )
        .filter(identities::Column::Id.eq(id))
        .exec(db)
        .await
        .context("recording a provider sign-in")?;
    Ok(())
}

/// The user's identities, oldest first.
pub async fn list_for_user(
    db: &impl ConnectionTrait,
    user_id: Uuid,
) -> anyhow::Result<Vec<identities::Model>> {
    identities::Entity::find()
        .filter(identities::Column::UserId.eq(user_id))
        .order_by_asc(identities::Column::CreatedAt)
        .order_by_asc(identities::Column::Id)
        .all(db)
        .await
        .context("listing identities")
}

/// Provider ids linked to each of `user_ids` (sorted, for the admin user list).
pub async fn provider_ids_by_user(
    db: &impl ConnectionTrait,
    user_ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, Vec<String>>> {
    let rows = identities::Entity::find()
        .filter(identities::Column::UserId.is_in(user_ids.iter().copied()))
        .all(db)
        .await
        .context("listing identities")?;
    let mut map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in rows {
        map.entry(row.user_id).or_default().push(row.provider_id);
    }
    for ids in map.values_mut() {
        ids.sort();
    }
    Ok(map)
}
