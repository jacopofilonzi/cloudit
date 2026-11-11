//! `update_check`: looks up the newest cloudit **server** version so admins learn when
//! their server is out of date (`GET /admin/summary`).
//!
//! - With `UPDATE_CHECK=false` it contacts nothing and only counts `disabled`.
//! - Server versions are git tags, not GitHub Releases: a `server-vX.Y.Z` tag only
//!   publishes the Docker image (the repository's GitHub Releases are the desktop
//!   app's, whose updater reads the "latest" one). So the job makes one request,
//!   `GET UPDATE_CHECK_URL` (default the GitHub API's "list matching references" for
//!   `tags/server-v`: every matching ref at once, `[{ref, node_id, url, object}]`, small
//!   enough for the 1 MiB body cap), through the shared outgoing client (`idp::http`:
//!   timeouts, no redirects, bounded body, `User-Agent: cloudit-server/<v>`).
//! - It keeps the refs named exactly `refs/tags/server-vX.Y.Z` (plain `x.y.z`, no
//!   suffix), picks the highest version and stores it in the `latest_release` setting
//!   with the server changelog as its page ([`SERVER_CHANGELOG_URL`]) and the time of
//!   the check. Running it again only overwrites that value (idempotent).
//! - No matching tag (an empty list), or a 404, means no server version is published
//!   (yet): nothing stored, the run succeeds. Any other failure (network, status, an
//!   answer that isn't a ref list) fails the run and keeps the previous value; the
//!   next run tries again.


use anyhow::Context as _;
use cloudit_core::protocol::LatestRelease;
use cloudit_core::version::Version;
use serde::Deserialize;

use super::{Counts, RunContext};
use crate::accounts::settings;
use crate::idp::http::HttpError;
use crate::state::AppState;

/// Tag prefix of server versions.
const SERVER_TAG_PREFIX: &str = "refs/tags/server-v";

/// The page stored with a found version: the server's release notes.
pub const SERVER_CHANGELOG_URL: &str =
    "https://github.com/jacopofilonzi/cloudit/blob/main/CHANGELOG-server.md";

/// The field read from the GitHub API's ref objects.
#[derive(Debug, Deserialize)]
struct GitRef {
    #[serde(rename = "ref")]
    name: String,
}

/// The version of a `refs/tags/server-vX.Y.Z` ref (exactly three numbers, no suffix).
fn server_version(name: &str) -> Option<Version> {
    let raw = name.strip_prefix(SERVER_TAG_PREFIX)?;
    let version = Version::parse(raw)?;
    // `Version::parse` also accepts a `v` and `-rc` / `+build` suffixes: a server tag
    // is the plain form only.
    (version.to_string() == raw).then_some(version)
}

/// The highest server version among the refs.
fn newest_server_version(refs: &[GitRef]) -> Option<Version> {
    refs.iter().filter_map(|r| server_version(&r.name)).max()
}

pub async fn run(state: &AppState, ctx: &RunContext, counts: &mut Counts) -> anyhow::Result<()> {
    if !state.config.update_check {
        counts.add("disabled", 1);
        return Ok(());
    }
    let url =
        reqwest::Url::parse(&state.config.update_check_url).context("parsing UPDATE_CHECK_URL")?;
    let refs: Vec<GitRef> = match state.idp.http.get_json(&url, None).await {
        Ok(refs) => refs,
        Err(HttpError::Status { status: 404, .. }) => {
            counts.add("no_release", 1);
            return Ok(());
        }
        Err(error) => return Err(error).context("fetching the server tags"),
    };
    let Some(version) = newest_server_version(&refs) else {
        counts.add("no_release", 1);
        return Ok(());
    };
    ctx.check()?;
    let latest = LatestRelease {
        version: version.to_string(),
        url: SERVER_CHANGELOG_URL.to_owned(),
        checked_at: crate::time::rfc3339(crate::time::now()),
    };
    settings::set_latest_release(&state.db, &latest).await?;
    counts.add("checked", 1);
    if cloudit_core::version::is_newer(&latest.version, env!("CARGO_PKG_VERSION")) {
        counts.add("newer", 1);
        tracing::info!(latest = %latest.version, "a newer cloudit server version is available");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_server_tags_count() {
        assert_eq!(
            server_version("refs/tags/server-v1.2.3"),
            Version::parse("1.2.3")
        );
        for name in [
            "refs/tags/v1.2.3",
            "refs/tags/app-v1.2.3",
            "refs/tags/server-1.2.3",
            "refs/tags/server-vv1.2.3",
            "refs/tags/server-v1.2.3-rc.1",
            "refs/tags/server-v1.2",
            "refs/tags/server-v01.2.3",
            "refs/heads/server-v1.2.3",
            "server-v1.2.3",
        ] {
            assert_eq!(server_version(name), None, "{name}");
        }
    }

    #[test]
    fn picks_the_highest_server_tag() {
        let refs = |names: &[&str]| -> Vec<GitRef> {
            names
                .iter()
                .map(|n| GitRef {
                    name: (*n).to_owned(),
                })
                .collect()
        };
        let version = newest_server_version(&refs(&[
            "refs/tags/app-v9.0.0",
            "refs/tags/server-v0.10.0",
            "refs/tags/server-v0.9.0",
            "refs/tags/server-v2.0.0-rc.1",
            "refs/tags/v5.0.0",
        ]))
        .unwrap();
        assert_eq!(version.to_string(), "0.10.0");
        assert!(newest_server_version(&refs(&["refs/tags/app-v1.0.0"])).is_none());
    }
}
