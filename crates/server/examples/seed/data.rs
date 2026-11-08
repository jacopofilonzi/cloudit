//! Demo folder contents: deterministic pseudo-random files that change a little
//! between "generations", so successive snapshots share most of their chunks.

use std::collections::BTreeMap;

/// Deterministic pseudo-random bytes (SplitMix64).
fn random(seed: u64, len: usize) -> Vec<u8> {
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

pub type Files = BTreeMap<String, Vec<u8>>;

/// A Minecraft-like world; `generation` changes some of the files.
pub fn world(generation: u64) -> Files {
    let mut files = Files::new();
    files.insert("level.dat".into(), random(10 + generation, 80 * 1024));
    files.insert(
        "region/r.0.0.mca".into(),
        random(20 + generation / 2, 600 * 1024),
    );
    files.insert("region/r.0.1.mca".into(), random(30, 300 * 1024));
    if generation >= 2 {
        files.insert(
            "region/r.1.0.mca".into(),
            random(40 + generation / 3, 400 * 1024),
        );
    }
    files.insert(
        "playerdata/steve.dat".into(),
        random(50 + generation, 4 * 1024),
    );
    files.insert("config.ini".into(), b"render=12\nsound=7\n".repeat(40));
    files
}

/// The same island seen from another computer: it kept playing offline.
pub fn island(generation: u64, offline: bool) -> Files {
    let mut files = Files::new();
    files.insert("island.dat".into(), random(100 + generation, 200 * 1024));
    files.insert("chests.json".into(), random(110, 30 * 1024));
    files.insert(
        "builds/castle.nbt".into(),
        random(120 + generation, 350 * 1024),
    );
    if offline {
        files.remove("chests.json");
        files.insert("builds/tower.nbt".into(), random(130, 120 * 1024));
        files.insert("island.dat".into(), random(140, 200 * 1024));
    }
    files
}

/// A small mod configuration.
pub fn mods() -> Files {
    let mut files = Files::new();
    files.insert(
        "mods.toml".into(),
        b"[mods]\nsodium=true\nlithium=true\n".repeat(20),
    );
    files.insert("options.txt".into(), random(150, 12 * 1024));
    files
}

/// The mods after Marco added one.
pub fn mods_v2() -> Files {
    let mut files = mods();
    files.insert(
        "mods/iris.toml".into(),
        b"iris=true
"
        .repeat(30),
    );
    files
}

/// The mods as Jacopo's laptop changed them.
pub fn mods_v3() -> Files {
    let mut files = mods();
    files.insert("options.txt".into(), random(160, 12 * 1024));
    files
}

/// Contracts: a few documents.
pub fn documents(generation: u64) -> Files {
    let mut files = Files::new();
    files.insert("affitto.pdf".into(), random(200, 500 * 1024));
    files.insert(
        "assicurazione.pdf".into(),
        random(210 + generation, 250 * 1024),
    );
    files
}

/// Holiday photos.
pub fn photos() -> Files {
    let mut files = Files::new();
    for n in 0..4 {
        files.insert(format!("IMG_{n:04}.jpg"), random(300 + n, 700 * 1024));
    }
    files
}
