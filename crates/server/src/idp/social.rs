//! OAuth2 providers without OIDC: GitHub and Discord. After the code exchange their
//! user APIs say who signed in:
//!
//! - GitHub: `GET /user` (subject = numeric `id`, name = `name` or `login`) and
//!   `GET /user/emails`: the email is the one marked primary **and** verified (none
//!   otherwise), so it is always verified when present.
//! - Discord: `GET /users/@me` (subject = `id`, name = `global_name` or `username`,
//!   `email` with its `verified` flag).
//!
//! Neither has groups. The endpoint URLs are fixed in production; tests replace them
//! with a fake server ([`SocialEndpoints`] can only be changed under `cfg(test)`).

use reqwest::Url;
use serde::Deserialize;

use super::claims::ExternalIdentity;
use super::http::{HttpClient, HttpError};
use cloudit_core::protocol::ProviderKind;

/// URLs of one OAuth2 provider.
#[derive(Debug, Clone)]
pub struct OAuth2Endpoints {
    pub authorize: Url,
    pub token: Url,
    /// API base, ending with `/`.
    pub api: Url,
}

/// GitHub and Discord endpoints.
#[derive(Debug, Clone)]
pub struct SocialEndpoints {
    pub github: OAuth2Endpoints,
    pub discord: OAuth2Endpoints,
}

impl SocialEndpoints {
    /// The real endpoints.
    pub fn production() -> anyhow::Result<Self> {
        let url = |s: &str| Url::parse(s).map_err(|e| anyhow::anyhow!("{s}: {e}"));
        Ok(Self {
            github: OAuth2Endpoints {
                authorize: url("https://github.com/login/oauth/authorize")?,
                token: url("https://github.com/login/oauth/access_token")?,
                api: url("https://api.github.com/")?,
            },
            discord: OAuth2Endpoints {
                authorize: url("https://discord.com/oauth2/authorize")?,
                token: url("https://discord.com/api/oauth2/token")?,
                api: url("https://discord.com/api/")?,
            },
        })
    }

    /// The endpoints of an OAuth2 kind; `None` for OIDC kinds.
    #[must_use]
    pub fn of(&self, kind: ProviderKind) -> Option<&OAuth2Endpoints> {
        match kind {
            ProviderKind::Github => Some(&self.github),
            ProviderKind::Discord => Some(&self.discord),
            _ => None,
        }
    }
}

fn api_url(base: &Url, path: &str) -> Result<Url, HttpError> {
    base.join(path).map_err(|e| HttpError::Request {
        url: base.to_string(),
        message: e.to_string(),
    })
}

#[derive(Deserialize)]
struct GithubUser {
    id: u64,
    login: String,
    name: Option<String>,
}

#[derive(Deserialize)]
struct GithubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

#[derive(Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
    global_name: Option<String>,
    email: Option<String>,
    verified: Option<bool>,
}

/// Who owns `access_token` at an OAuth2 provider.
pub async fn fetch_identity(
    http: &HttpClient,
    kind: ProviderKind,
    endpoints: &OAuth2Endpoints,
    access_token: &str,
) -> Result<ExternalIdentity, HttpError> {
    match kind {
        ProviderKind::Discord => {
            let user: DiscordUser = http
                .get_json(&api_url(&endpoints.api, "users/@me")?, Some(access_token))
                .await?;
            Ok(ExternalIdentity::new(
                user.id,
                user.email.as_deref(),
                user.verified.unwrap_or(false),
                &[user.global_name.as_deref(), Some(&user.username)],
                None,
            ))
        }
        _ => {
            let user: GithubUser = http
                .get_json(&api_url(&endpoints.api, "user")?, Some(access_token))
                .await?;
            let emails: Vec<GithubEmail> = http
                .get_json(&api_url(&endpoints.api, "user/emails")?, Some(access_token))
                .await?;
            let email = emails.iter().find(|e| e.primary && e.verified);
            Ok(ExternalIdentity::new(
                user.id.to_string(),
                email.map(|e| e.email.as_str()),
                email.is_some(),
                &[user.name.as_deref(), Some(&user.login)],
                None,
            ))
        }
    }
}
