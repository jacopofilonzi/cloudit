//! `cloudit-server` subcommands besides serving. They read the same configuration as
//! the server and connect to its database, but never migrate it nor start the HTTP
//! server, so they are safe to run next to running instances (`docker compose exec`).
//!
//! - `setup-token`: prints the pending first-run setup token, or says setup is done.
//! - `admin reset-password <email>`: emergency password-reset link for an account (also
//!   one without a password), printed as a path and, with `PUBLIC_URL`, a full URL.
//! - `gen-secret-key`: prints a new random `SECRET_KEY` (needs no configuration).
//! - `healthcheck`: the Docker probe (see [`crate::healthcheck`]).

use std::io::Write as _;

use anyhow::{Context as _, bail};

use crate::accounts::setup::SetupStatus;
use crate::accounts::{password_resets, setup, users};
use crate::config::Config;
use crate::db;

pub const USAGE: &str = "\
usage: cloudit-server [COMMAND]

Without a command, runs the server. Commands:
  setup-token                    print the pending first-run setup token
  admin reset-password <email>   create a password-reset link for an account (24 h)
  gen-secret-key                 print a new random SECRET_KEY (base64 of 32 bytes)
  healthcheck                    probe GET /health on 127.0.0.1:$PORT (Docker HEALTHCHECK)
  help                           show this help
";

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Serve,
    Healthcheck,
    SetupToken,
    GenSecretKey,
    AdminResetPassword { email: String },
    Help,
}

/// Parses the arguments after the program name.
pub fn parse(args: &[String]) -> anyhow::Result<Command> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(match args.as_slice() {
        [] => Command::Serve,
        ["healthcheck"] => Command::Healthcheck,
        ["setup-token"] => Command::SetupToken,
        ["gen-secret-key"] => Command::GenSecretKey,
        ["admin", "reset-password", email] => Command::AdminResetPassword {
            email: (*email).to_owned(),
        },
        ["help" | "--help" | "-h"] => Command::Help,
        _ => bail!("unknown command line: {}\n\n{USAGE}", args.join(" ")),
    })
}

fn print(text: &str) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .context("writing to stdout")?;
    out.flush().context("writing to stdout")
}

const NOT_INITIALIZED: &str =
    "the database has no setup state: start the server once so it creates its tables";

/// `setup-token`.
pub async fn setup_token(config: &Config) -> anyhow::Result<()> {
    print(&setup_token_text(config).await?)
}

/// The output of `setup-token`: the token alone on a line, so scripts can capture it.
pub async fn setup_token_text(config: &Config) -> anyhow::Result<String> {
    let db = db::connect(config).await?;
    let status = setup::status(&db).await.context(NOT_INITIALIZED);
    db.close().await.context("closing the database")?;
    match status? {
        SetupStatus::Pending(token) => Ok(format!("{}\n", token.expose())),
        SetupStatus::Completed => Ok("setup is already completed\n".to_owned()),
        SetupStatus::NotInitialized => bail!(NOT_INITIALIZED),
    }
}

/// `admin reset-password <email>`.
pub async fn reset_password(config: &Config, email: &str) -> anyhow::Result<()> {
    print(&reset_password_text(config, email).await?)
}

/// The output of `admin reset-password`.
pub async fn reset_password_text(config: &Config, email: &str) -> anyhow::Result<String> {
    let email = crate::validate::email(email, "email")
        .map_err(|_| anyhow::anyhow!("{email:?} is not a valid email address"))?;
    let db = db::connect(config).await?;
    let user = users::find_by_email(&db, &email)
        .await
        .context(NOT_INITIALIZED)?
        .with_context(|| format!("no account with email {email}"))?;
    let (token, reset) = password_resets::create(&db, user.id, None).await?;
    let public_url = crate::state::effective_public_url(config, &db).await;
    db.close().await.context("closing the database")?;
    let path = format!("/reset-password/{}", token.expose());
    let mut text = format!(
        "password-reset link for {email} (single use, expires {}):\n  path: {path}\n",
        crate::time::rfc3339(reset.expires_at)
    );
    match public_url? {
        Some(base) => text.push_str(&format!("  url:  {base}{path}\n")),
        None => text.push_str(
            "  (set the public URL to get a full URL; append the path to the server address)\n",
        ),
    }
    Ok(text)
}

/// `gen-secret-key`: the key alone on a line.
pub fn gen_secret_key() -> anyhow::Result<()> {
    print(&format!(
        "{}\n",
        crate::crypto::SecretKey::generate()?.encode()
    ))
}

/// Prints the help text.
pub fn help() -> anyhow::Result<()> {
    print(USAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse(&args(&[])).unwrap(), Command::Serve);
        assert_eq!(
            parse(&args(&["healthcheck"])).unwrap(),
            Command::Healthcheck
        );
        assert_eq!(parse(&args(&["setup-token"])).unwrap(), Command::SetupToken);
        assert_eq!(
            parse(&args(&["gen-secret-key"])).unwrap(),
            Command::GenSecretKey
        );
        assert_eq!(
            parse(&args(&["admin", "reset-password", "a@b.c"])).unwrap(),
            Command::AdminResetPassword {
                email: "a@b.c".to_owned()
            }
        );
        assert_eq!(parse(&args(&["--help"])).unwrap(), Command::Help);
        assert!(parse(&args(&["admin", "reset-password"])).is_err());
        assert!(parse(&args(&["serve", "extra"])).is_err());
    }
}
