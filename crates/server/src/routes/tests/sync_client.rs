//! A minimal sync client for the tests, built on `cloudit-core` the way the desktop
//! client will use it: chunk a folder in memory, ask which chunks are missing, upload
//! them through the proxy.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use cloudit_core::chunk::chunker::chunk_slice;
use cloudit_core::chunk::codec::{decode_chunk, encode_chunk};
use cloudit_core::hash::ChunkHash;
use cloudit_core::manifest::{
    ChunkRef, DEFAULT_MAX_MANIFEST_BYTES, EntryKind, Manifest, ManifestEntry, Timestamp,
    decode_manifest,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::support::{As, Resp, TestApp};
use crate::db::entities::scopes;
use crate::sync::usage;

/// Deterministic pseudo-random bytes (SplitMix64).
pub fn pseudo_random(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        out.extend_from_slice(&(z ^ (z >> 31)).to_le_bytes());
    }
    out.truncate(len);
    out
}

/// One entry of a [`Tree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    File {
        data: Vec<u8>,
        mtime: Timestamp,
        executable: bool,
    },
    /// An empty directory.
    Dir { mtime: Timestamp },
}

/// A folder in memory, by path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree(pub BTreeMap<String, Item>);

pub fn ts(secs: i64, nanos: u32) -> Timestamp {
    Timestamp { secs, nanos }
}

impl Tree {
    /// About 4.5 MiB: several multi-chunk files, a small and an empty file, an
    /// executable and an empty directory.
    pub fn sample() -> Self {
        let mut tree = Self::default();
        tree.file("saves/slot1.sav", pseudo_random(1, 3 * 1024 * 1024));
        tree.file("saves/slot2.sav", pseudo_random(2, 700 * 1024));
        // Compressible: stored with zstd.
        tree.file("config.ini", b"volume=7\n".repeat(300));
        tree.file("empty.txt", Vec::new());
        tree.0.insert(
            "bin/run.sh".to_owned(),
            Item::File {
                data: b"#!/bin/sh\necho hi\n".to_vec(),
                mtime: ts(1_700_000_100, 0),
                executable: true,
            },
        );
        tree.0.insert(
            "screenshots".to_owned(),
            Item::Dir {
                mtime: ts(1_700_000_200, 0),
            },
        );
        tree
    }

    pub fn file(&mut self, path: &str, data: Vec<u8>) {
        self.0.insert(
            path.to_owned(),
            Item::File {
                data,
                mtime: ts(1_700_000_000, 123_456_789),
                executable: false,
            },
        );
    }

    /// The manifest and the raw chunks it references.
    pub fn manifest(&self) -> (Manifest, BTreeMap<ChunkHash, Vec<u8>>) {
        let mut chunks = BTreeMap::new();
        let mut entries = Vec::new();
        for (path, item) in &self.0 {
            match item {
                Item::File {
                    data,
                    mtime,
                    executable,
                } => {
                    let refs = chunk_slice(data)
                        .map(|c| {
                            chunks.insert(c.hash, c.data.to_vec());
                            ChunkRef {
                                hash: c.hash,
                                size: u32::try_from(c.data.len()).unwrap(),
                            }
                        })
                        .collect();
                    entries.push(ManifestEntry::file(path, *mtime, *executable, refs));
                }
                Item::Dir { mtime } => entries.push(ManifestEntry::dir(path, *mtime)),
            }
        }
        (Manifest::new(entries).unwrap(), chunks)
    }
}

/// The path of a transfer URL (absolute or not) for the test router.
pub fn local_path(url: &str) -> &str {
    let at = url.find("/api/v1/").expect("an API URL");
    &url[at..]
}

/// One client (credentials) syncing one folder.
pub struct SyncClient<'a> {
    pub app: &'a TestApp,
    pub who: As,
    pub folder: String,
}

impl<'a> SyncClient<'a> {
    pub fn new(app: &'a TestApp, who: &As, folder: &str) -> Self {
        Self {
            app,
            who: who.clone(),
            folder: folder.to_owned(),
        }
    }

    pub fn url(&self, rest: &str) -> String {
        format!("/api/v1/folders/{}{rest}", self.folder)
    }

    /// `POST chunks/check` for these chunks.
    pub async fn check(&self, chunks: &BTreeMap<ChunkHash, Vec<u8>>) -> Resp {
        let list: Vec<Value> = chunks
            .iter()
            .map(|(hash, data)| json!({ "hash": hash.to_string(), "size": data.len() }))
            .collect();
        self.app
            .post(
                &self.url("/chunks/check"),
                &self.who,
                json!({ "chunks": list }),
            )
            .await
    }

    /// Uploads every chunk a check answer lists, through its (proxy) transfer.
    pub async fn upload(&self, check: &Resp, chunks: &BTreeMap<ChunkHash, Vec<u8>>) -> usize {
        check.assert(StatusCode::OK);
        let missing = check.body["missing"].as_array().unwrap();
        for item in missing {
            let hash: ChunkHash = item["hash"].as_str().unwrap().parse().unwrap();
            let upload = &item["upload"];
            assert_eq!(upload["kind"], "proxy");
            assert_eq!(upload["method"], "PUT");
            let url = upload["url"].as_str().unwrap();
            let encoded = encode_chunk(&chunks[&hash]).unwrap();
            self.app
                .send_raw("PUT", local_path(url), &self.who, encoded)
                .await
                .assert(StatusCode::NO_CONTENT);
        }
        missing.len()
    }

    /// `POST snapshots?parent=…&kind=…` with stored manifest bytes.
    pub async fn commit_raw(&self, stored: Vec<u8>, parent: Option<&str>, kind: &str) -> Resp {
        let uri = self.url(&format!(
            "/snapshots?parent={}&kind={kind}",
            parent.unwrap_or("none")
        ));
        self.app.send_raw("POST", &uri, &self.who, stored).await
    }

    /// Commits `manifest` on `parent`.
    pub async fn commit(&self, manifest: &Manifest, parent: Option<&str>, kind: &str) -> Resp {
        self.commit_raw(manifest.encode().unwrap().compressed, parent, kind)
            .await
    }

    /// Uploads what is missing of `tree` and commits it as `normal` on `parent`;
    /// returns the new snapshot id.
    pub async fn sync(&self, tree: &Tree, parent: Option<&str>) -> String {
        let (manifest, chunks) = tree.manifest();
        self.push_chunks(&chunks).await;
        let resp = self.commit(&manifest, parent, "normal").await;
        resp.assert(StatusCode::CREATED);
        resp.body["snapshot"]["id"].as_str().unwrap().to_owned()
    }

    /// Downloads a snapshot the way a client restores it: the stored manifest, then
    /// every chunk through its download transfer, decoded and verified.
    pub async fn download(&self, snapshot: &str) -> Tree {
        let raw = self
            .app
            .get_bytes(&format!("/api/v1/snapshots/{snapshot}/manifest"), &self.who)
            .await;
        raw.assert(StatusCode::OK);
        let (manifest, hash) = decode_manifest(&raw.body, DEFAULT_MAX_MANIFEST_BYTES).unwrap();
        assert_eq!(raw.headers["etag"], format!("\"{hash}\""));
        let hashes: Vec<String> = manifest
            .chunk_hashes()
            .iter()
            .map(ToString::to_string)
            .collect();
        let resp = self
            .app
            .post(
                &self.url("/chunks/download"),
                &self.who,
                json!({ "hashes": hashes }),
            )
            .await;
        resp.assert(StatusCode::OK);
        assert_eq!(resp.body["missing"], json!([]));
        let mut chunks = BTreeMap::new();
        for item in resp.body["chunks"].as_array().unwrap() {
            let hash: ChunkHash = item["hash"].as_str().unwrap().parse().unwrap();
            let url = item["download"]["url"].as_str().unwrap();
            let stored = self.app.get_bytes(local_path(url), &self.who).await;
            stored.assert(StatusCode::OK);
            chunks.insert(hash, decode_chunk(&stored.body, &hash).unwrap());
        }
        let mut tree = Tree::default();
        for entry in manifest.entries {
            let item = match entry.kind {
                EntryKind::Dir => Item::Dir { mtime: entry.mtime },
                EntryKind::File => Item::File {
                    data: entry
                        .chunks
                        .iter()
                        .flat_map(|c| chunks[&c.hash].iter().copied())
                        .collect(),
                    mtime: entry.mtime,
                    executable: entry.executable,
                },
            };
            tree.0.insert(entry.path, item);
        }
        tree
    }

    /// Check + upload; the number of chunks uploaded.
    pub async fn push_chunks(&self, chunks: &BTreeMap<ChunkHash, Vec<u8>>) -> usize {
        let check = self.check(chunks).await;
        self.upload(&check, chunks).await
    }
}

/// The scope's usage counter and the value recomputed from its rows.
pub async fn usage_of(app: &TestApp, scope: &str) -> (u64, u64) {
    let id: Uuid = scope.parse().unwrap();
    let scope = <scopes::Entity as sea_orm::EntityTrait>::find_by_id(id)
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    let recomputed = usage::recompute(&app.state.db, id).await.unwrap();
    (usage::bytes(scope.stored_bytes), recomputed)
}

/// Asserts the counter matches the rows; returns it.
#[track_caller]
pub fn consistent(values: (u64, u64)) -> u64 {
    assert_eq!(values.0, values.1, "stored_bytes vs recomputed");
    values.0
}
