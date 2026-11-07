//! ZIP download of a snapshot, streamed: a task writes the archive (`async_zip`) into
//! one end of an in-memory pipe while the response body reads the other, so memory
//! stays bounded (the pipe, a few prefetched chunks) whatever the snapshot's size.
//!
//! - Entries in manifest order. Files are deflated and written with data descriptors
//!   (sizes and CRC follow the data), which also makes every entry ZIP64-capable;
//!   files of no bytes and empty directories (`path/`) are stored whole.
//! - Each chunk is read from storage and decoded with `cloudit-core`, which verifies
//!   its hash: a damaged chunk aborts the download instead of producing wrong bytes.
//! - Metadata: the DOS timestamp (2 s precision, years 1980 to 2107, written as UTC:
//!   the format has no time zone); Unix modes `0644` / `0755` (the executable bit) and
//!   `0755` for directories. `async_zip` 0.0.19 doesn't expose custom extra fields, so
//!   there is no "extended timestamp" (0x5455) with full precision.
//! - A failure mid-stream ends the body with an error, so the client sees an aborted
//!   transfer rather than a truncated archive that looks complete.

use anyhow::Context as _;
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipDateTime, ZipDateTimeBuilder, ZipEntryBuilder};
use axum::body::Body;
use bytes::{Bytes, BytesMut};
use chrono::{DateTime, Datelike as _, Timelike as _, Utc};
use cloudit_core::chunk::codec::decode_chunk;
use cloudit_core::manifest::{ChunkRef, EntryKind, Manifest, ManifestEntry, Timestamp};
use futures::{AsyncWriteExt as _, StreamExt as _};
use tokio::io::{AsyncReadExt as _, DuplexStream};
use uuid::Uuid;

use super::keys;
use crate::state::AppState;

/// Bytes buffered between the writer task and the response.
const PIPE_BYTES: usize = 256 * 1024;
/// Chunks fetched and decoded ahead of the writer.
const PREFETCH: usize = 4;

/// Unix mode of a regular file (`S_IFREG`) / directory (`S_IFDIR`).
const MODE_FILE: u16 = 0o100_644;
const MODE_EXEC: u16 = 0o100_755;
const MODE_DIR: u16 = 0o040_755;

/// `<folder name>-<YYYYMMDD-HHMMSS>.zip` (snapshot time, UTC).
#[must_use]
pub fn filename(folder: &str, created: DateTime<Utc>) -> String {
    format!("{folder}-{}.zip", created.format("%Y%m%d-%H%M%S"))
}

/// `Content-Disposition` for `name`: an ASCII fallback (non-ASCII, quotes, `\` and
/// control characters as `_`) plus the RFC 5987 UTF-8 form.
#[must_use]
pub fn content_disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::with_capacity(name.len() * 3);
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

fn dos_time(mtime: Timestamp) -> ZipDateTime {
    let t = DateTime::from_timestamp(mtime.secs, 0).unwrap_or_default();
    let builder = ZipDateTimeBuilder::new();
    let builder = if t.year() < 1980 {
        builder.year(1980).month(1).day(1)
    } else if t.year() > 2107 {
        builder
            .year(2107)
            .month(12)
            .day(31)
            .hour(23)
            .minute(59)
            .second(58)
    } else {
        builder
            .year(t.year())
            .month(t.month())
            .day(t.day())
            .hour(t.hour())
            .minute(t.minute())
            .second(t.second())
    };
    builder.build()
}

fn entry(entry: &ManifestEntry) -> ZipEntryBuilder {
    let (name, compression, mode) = match entry.kind {
        EntryKind::Dir => (format!("{}/", entry.path), Compression::Stored, MODE_DIR),
        EntryKind::File if entry.chunks.is_empty() => (
            entry.path.clone(),
            Compression::Stored,
            if entry.executable {
                MODE_EXEC
            } else {
                MODE_FILE
            },
        ),
        EntryKind::File => (
            entry.path.clone(),
            Compression::Deflate,
            if entry.executable {
                MODE_EXEC
            } else {
                MODE_FILE
            },
        ),
    };
    ZipEntryBuilder::new(name.into(), compression)
        .last_modification_date(dos_time(entry.mtime))
        .unix_permissions(mode)
}

/// Reads and decodes one chunk (hash verified).
async fn chunk(state: &AppState, scope: Uuid, chunk: ChunkRef) -> anyhow::Result<Vec<u8>> {
    let key = keys::chunk(scope, &chunk.hash);
    let stored = state
        .blobs
        .get_bytes(&key)
        .await?
        .with_context(|| format!("chunk object {key} is missing"))?;
    tokio::task::spawn_blocking(move || decode_chunk(&stored, &chunk.hash))
        .await
        .context("chunk decoding task")?
        .with_context(|| format!("chunk object {key} is damaged"))
}

async fn write(
    state: AppState,
    scope: Uuid,
    manifest: Manifest,
    out: DuplexStream,
) -> anyhow::Result<()> {
    let mut zip = ZipFileWriter::with_tokio(out);
    for item in &manifest.entries {
        let builder = entry(item);
        if item.chunks.is_empty() {
            zip.write_entry_whole(builder, &[])
                .await
                .with_context(|| format!("writing {}", item.path))?;
            continue;
        }
        let mut writer = zip
            .write_entry_stream(builder)
            .await
            .with_context(|| format!("starting {}", item.path))?;
        let state = &state;
        let mut parts = futures::stream::iter(item.chunks.iter().copied())
            .map(|c| chunk(state, scope, c))
            .buffered(PREFETCH);
        while let Some(part) = parts.next().await {
            writer
                .write_all(&part?)
                .await
                .with_context(|| format!("writing {}", item.path))?;
        }
        writer
            .close()
            .await
            .with_context(|| format!("finishing {}", item.path))?;
    }
    zip.close().await.context("writing the central directory")?;
    Ok(())
}

/// The archive of `manifest` (a snapshot of `scope`) as a streaming response body.
pub fn body(state: AppState, scope: Uuid, manifest: Manifest) -> Body {
    let (out, input) = tokio::io::duplex(PIPE_BYTES);
    let task = tokio::spawn(write(state, scope, manifest, out));

    enum Stage {
        Reading(DuplexStream, tokio::task::JoinHandle<anyhow::Result<()>>),
        Done,
    }
    let stream = futures::stream::unfold(Stage::Reading(input, task), |stage| async move {
        let Stage::Reading(mut input, task) = stage else {
            return None;
        };
        let mut buf = BytesMut::with_capacity(64 * 1024);
        match input.read_buf(&mut buf).await {
            Ok(0) => {
                // The writer is done (or failed): surface its error, if any.
                let outcome = match task.await {
                    Ok(Ok(())) => return None,
                    Ok(Err(err)) => err,
                    Err(join) => anyhow::Error::new(join).context("archive task"),
                };
                tracing::error!(error = ?outcome, "zip download failed");
                Some((Err(std::io::Error::other("archive failed")), Stage::Done))
            }
            Ok(_) => Some((
                Ok::<Bytes, std::io::Error>(buf.freeze()),
                Stage::Reading(input, task),
            )),
            Err(err) => {
                task.abort();
                Some((Err(err), Stage::Done))
            }
        }
    });
    Body::from_stream(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_headers() {
        let t = DateTime::parse_from_rfc3339("2026-11-07T09:05:03Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(filename("Saves", t), "Saves-20261107-090503.zip");
        assert_eq!(
            content_disposition("Salvataggi è \"x\".zip"),
            "attachment; filename=\"Salvataggi _ _x_.zip\"; \
             filename*=UTF-8''Salvataggi%20%C3%A8%20%22x%22.zip"
        );
    }

    #[test]
    fn dos_times_are_clamped() {
        let at = |secs| dos_time(Timestamp { secs, nanos: 0 });
        assert_eq!(at(0).year(), 1980);
        let t = at(1_700_000_001);
        assert_eq!((t.year(), t.month(), t.day()), (2023, 11, 14));
        assert_eq!((t.hour(), t.minute(), t.second()), (22, 13, 20));
        assert_eq!(at(i64::MAX).year(), 1980, "out of chrono's range");
        assert_eq!(at(5_000_000_000).year(), 2107);
    }
}
