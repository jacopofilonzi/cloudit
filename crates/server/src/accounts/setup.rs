//! First-run setup: the `setup_state` row (id 1) and its one-time token.
//!
//! - At startup ([`init`]), if no user exists, every instance runs
//!   `INSERT … (1, <random token>, now) ON CONFLICT (id) DO NOTHING`, so exactly one
//!   token wins, then reads the row back and, while setup is pending, logs that token
//!   at WARN: all instances print the same one. `SETUP_TOKEN` replaces the stored token
//!   while setup is pending.
//! - [`complete`] consumes the token with one conditional statement,
//!   `UPDATE setup_state SET token = NULL, completed_at = now WHERE id = 1 AND
//!   completed_at IS NULL AND token = ?`: exactly one concurrent caller gets the row.
//! - The row is never deleted: once completed, no instance can create a new token.
//!
//! The token is stored in plaintext so `cloudit-server setup-token` can print it; it is
//! read into a [`Secret`] and logged only by [`init`], on purpose.

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict, Query};
use sea_orm::{ConnectionTrait, DeriveIden, StatementBuilder};

use crate::auth::tokens;
use crate::config::Secret;
use crate::errors::ApiError;

#[derive(DeriveIden)]
enum SetupState {
    Table,
    Id,
    Token,
    CreatedAt,
    CompletedAt,
}

/// The single row's id.
const ROW_ID: i32 = 1;

/// State of first-run setup.
#[derive(Debug, PartialEq, Eq)]
pub enum SetupStatus {
    /// No row: the server never started on this database (or it predates setup).
    NotInitialized,
    /// Waiting for `POST /api/v1/setup` with this token.
    Pending(Secret),
    Completed,
}

impl SetupStatus {
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_))
    }
}

async fn execute(db: &impl ConnectionTrait, stmt: &impl StatementBuilder) -> anyhow::Result<u64> {
    Ok(db.execute(stmt).await?.rows_affected())
}

/// Reads the setup row.
pub async fn status(db: &impl ConnectionTrait) -> anyhow::Result<SetupStatus> {
    let stmt = Query::select()
        .columns([SetupState::Token, SetupState::CompletedAt])
        .from(SetupState::Table)
        .and_where(Expr::col(SetupState::Id).eq(ROW_ID))
        .to_owned();
    let Some(row) = db.query_one(&stmt).await.context("reading setup_state")? else {
        return Ok(SetupStatus::NotInitialized);
    };
    let token: Option<String> = row.try_get("", "token").context("setup_state.token")?;
    let completed: Option<DateTime<Utc>> = row
        .try_get("", "completed_at")
        .context("setup_state.completed_at")?;
    Ok(match (completed, token) {
        (None, Some(token)) => SetupStatus::Pending(Secret::new(token)),
        _ => SetupStatus::Completed,
    })
}

/// Startup step (see the module docs). Returns the resulting status.
pub async fn init(
    db: &impl ConnectionTrait,
    env_token: Option<&Secret>,
) -> anyhow::Result<SetupStatus> {
    if !super::users::any_exists(db).await? {
        let token = match env_token {
            Some(token) => token.clone(),
            None => tokens::new_token()?,
        };
        let insert = Query::insert()
            .into_table(SetupState::Table)
            .columns([SetupState::Id, SetupState::Token, SetupState::CreatedAt])
            .values([
                ROW_ID.into(),
                token.expose().into(),
                crate::time::now().into(),
            ])
            .context("building the setup_state insert")?
            .on_conflict(OnConflict::column(SetupState::Id).do_nothing().to_owned())
            .to_owned();
        // `DO NOTHING` affects 0 rows when another instance won: not an error.
        execute(db, &insert)
            .await
            .context("creating the setup token")?;
    }
    if let Some(token) = env_token {
        let update = Query::update()
            .table(SetupState::Table)
            .value(SetupState::Token, token.expose())
            .and_where(Expr::col(SetupState::Id).eq(ROW_ID))
            .and_where(Expr::col(SetupState::CompletedAt).is_null())
            .to_owned();
        execute(db, &update).await.context("applying SETUP_TOKEN")?;
    }
    let status = status(db).await?;
    if let SetupStatus::Pending(token) = &status {
        // The one place a secret is logged, on purpose: the operator needs it to create
        // the first admin. It stops working once setup is done.
        tracing::warn!(
            setup_token = %token.expose(),
            "first-run setup pending: open the web UI and create the first admin with this \
             token (also shown by `cloudit-server setup-token`)"
        );
    }
    Ok(status)
}

/// Consumes the setup token (call inside the transaction that creates the first admin;
/// make it the first statement). Errors: `setup_already_done`, `setup_token_invalid`.
pub async fn complete(db: &impl ConnectionTrait, token: &str) -> Result<(), ApiError> {
    let update = Query::update()
        .table(SetupState::Table)
        .value(SetupState::Token, Option::<String>::None)
        .value(SetupState::CompletedAt, crate::time::now())
        .and_where(Expr::col(SetupState::Id).eq(ROW_ID))
        .and_where(Expr::col(SetupState::CompletedAt).is_null())
        .and_where(Expr::col(SetupState::Token).eq(token))
        .to_owned();
    if execute(db, &update).await.context("completing setup")? == 1 {
        return Ok(());
    }
    // Lost: say why (the token can't be guessed through this, the update already
    // failed).
    Err(match status(db).await? {
        SetupStatus::Pending(_) => ApiError::SetupTokenInvalid,
        SetupStatus::Completed | SetupStatus::NotInitialized => ApiError::SetupAlreadyDone,
    })
}

/// Checks a token against the pending one in constant time, without consuming it:
/// lets `POST /setup` fail fast before hashing the password.
pub async fn check_token(db: &impl ConnectionTrait, token: &str) -> Result<(), ApiError> {
    match status(db).await? {
        SetupStatus::Pending(expected) if tokens::constant_time_eq(expected.expose(), token) => {
            Ok(())
        }
        SetupStatus::Pending(_) => Err(ApiError::SetupTokenInvalid),
        SetupStatus::Completed | SetupStatus::NotInitialized => Err(ApiError::SetupAlreadyDone),
    }
}
