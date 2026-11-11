//! `update_check`: looks up the newest published cloudit **server** release (daily) so
//! admins learn when their server is out of date (`GET /admin/summary`).
//!
//! - With `UPDATE_CHECK=false` it contacts nothing and only counts `disabled`.
//! - `GET UPDATE_CHECK_URL` (default the GitHub API's "list releases" of the project,
//!   newest first, one page of 30: GitHub release objects are large and the body is
//!   capped at 1 MiB) through the shared outgoing client (`idp::http`: timeouts, no
//!   redirects, bounded body, `User-Agent: cloudit-server/<v>`).
//! - The server and the desktop app are released separately (`server-vX.Y.Z` /
//!   `app-vX.Y.Z` tags), and GitHub's "latest release" is always an app release (the
//!   desktop updater reads it), so the job picks the highest version among the
//!   published (not draft, not pre-release) releases tagged `server-vX.Y.Z`; other
//!   tags are ignored. Its version and `html_url` (the release page) are stored in the
//!   `latest_release` setting with the time of the check. Running it again only
//!   overwrites that value (idempotent).
//! - No server release in the list, or a 404, means none is published (yet): nothing
//!   stored, the run succeeds. Any other failure (network, status, an answer that
//!   isn't a release list, a non-http(s) page) fails the run and keeps the previous
//!   value; the next run tries again.

use anyhow::Context as _;
use cloudit_core::protocol::LatestRelease;
use cloudit_core::version::Version;
use serde::Deserialize;

use super::{Counts, RunContext};
use crate::accounts::settings;
use crate::idp::http::HttpError;
use crate::state::AppState;

/// Tag prefix of server releases.
const SERVER_TAG_PREFIX: &str = "server-v";

/// The fields read from the GitHub API's release objects.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The version of a `server-vX.Y.Z` tag (exactly three numbers, no suffix).
fn server_version(tag: &str) -> Option<Version> {
    let raw = tag.strip_prefix(SERVER_TAG_PREFIX)?;
    let version = Version::parse(raw)?;
    // `Version::parse` also accepts a `v` and `-rc` / `+build` suffixes: a server tag
    // is the plain form only.
    (version.to_string() == raw).then_some(version)
}

/// The newest published server release of a list.
fn newest_server_release(releases: Vec<GithubRelease>) -> Option<(Version, GithubRelease)> {
    releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| server_version(&r.tag_name).map(|v| (v, r)))
        .max_by_key(|(v, _)| *v)
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    if !state.config.update_check {
        counts.add("disabled", 1);
        return Ok(());
    }
    let url =
        reqwest::Url::parse(&state.config.update_check_url).context("parsing UPDATE_CHECK_URL")?;
    let releases: Vec<GithubRelease> = match state.idp.http.get_json(&url, None).await {
        Ok(releases) => releases,
        Err(HttpError::Status { status: 404, .. }) => {
            counts.add("no_release", 1);
            return Ok(());
        }
        Err(error) => return Err(error).context("fetching the releases"),
    };
    let Some((version, release)) = newest_server_release(releases) else {
        counts.add("no_release", 1);
        return Ok(());
    };
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
        tracing::info!(latest = %latest.version, "a newer cloudit server release is available");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_server_tags_count() {
        assert_eq!(server_version("server-v1.2.3"), Version::parse("1.2.3"));
        for tag in [
            "v1.2.3",
            "app-v1.2.3",
            "server-1.2.3",
            "server-vv1.2.3",
            "server-v1.2.3-rc.1",
            "server-v1.2",
            "server-v01.2.3",
        ] {
            assert_eq!(server_version(tag), None, "{tag}");
        }
    }

    #[test]
    fn picks_the_highest_published_server_release() {
        let release = |tag: &str, draft, prerelease| GithubRelease {
            tag_name: tag.to_owned(),
            html_url: format!("https://example.test/{tag}"),
            draft,
            prerelease,
        };
        let (version, picked) = newest_server_release(vec![
            release("app-v9.0.0", false, false),
            release("server-v0.10.0", false, false),
            release("server-v0.9.0", false, false),
            release("server-v1.0.0", true, false),
            release("server-v2.0.0", false, true),
            release("v5.0.0", false, false),
        ])
        .unwrap();
        assert_eq!(version.to_string(), "0.10.0");
        assert_eq!(picked.tag_name, "server-v0.10.0");
        assert!(newest_server_release(vec![release("app-v1.0.0", false, false)]).is_none());
    }
}
