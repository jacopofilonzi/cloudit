//! `update_check`: looks up the latest published cloudit release (daily) so admins
//! learn when their server is out of date (`GET /admin/summary`).
//!
//! - With `UPDATE_CHECK=false` it contacts nothing and only counts `disabled`.
//! - `GET UPDATE_CHECK_URL` (default the GitHub API's "latest release" of the
//!   project) through the shared outgoing client (`idp::http`: timeouts, no
//!   redirects, bounded body, `User-Agent: cloudit-server/<v>`); the answer's
//!   `tag_name` (`v1.2.3`) and `html_url` (the release page) are stored in the
//!   `latest_release` setting with the time of the check. Running it again only
//!   overwrites that value (idempotent).
//! - A 404 means no release is published yet: nothing stored, the run succeeds.
//!   Any other failure (network, status, a tag that isn't `x.y.z`, a non-http(s) page)
//!   fails the run and keeps the previous value; the next run tries again.

use anyhow::Context as _;
use cloudit_core::protocol::LatestRelease;
use cloudit_core::version::Version;
use serde::Deserialize;

use super::{Counts, RunContext};
use crate::accounts::settings;
use crate::idp::http::HttpError;
use crate::state::AppState;

/// The fields read from the GitHub API's release object.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    if !state.config.update_check {
        counts.add("disabled", 1);
        return Ok(());
    }
    let url =
        reqwest::Url::parse(&state.config.update_check_url).context("parsing UPDATE_CHECK_URL")?;
    let release: GithubRelease = match state.idp.http.get_json(&url, None).await {
        Ok(release) => release,
        Err(HttpError::Status { status: 404, .. }) => {
            counts.add("no_release", 1);
            return Ok(());
        }
        Err(error) => return Err(error).context("fetching the latest release"),
    };
    let version = Version::parse(&release.tag_name)
        .with_context(|| format!("release tag {:?} is not a version", release.tag_name))?;
    let page = url::Url::parse(&release.html_url)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .with_context(|| format!("release page {:?} is not an http(s) URL", release.html_url))?;
    ctx.check()?;
    let latest = LatestRelease {
        version: version.to_string(),
        url: page.to_string(),
        checked_at: crate::time::rfc3339(crate::time::now()),
    };
    settings::set_latest_release(&state.db, &latest).await?;
    counts.add("checked", 1);
    if cloudit_core::version::is_newer(&latest.version, env!("CARGO_PKG_VERSION")) {
        counts.add("newer", 1);
        tracing::info!(latest = %latest.version, "a newer cloudit release is available");
    }
    Ok(())
}
