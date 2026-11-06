//! Test helpers: deterministic pseudo-random data (no `rand` dependency).

/// `len` bytes from a SplitMix64 generator seeded with `seed`. The output for a given seed
/// must never change: golden vectors depend on it.
pub fn pseudo_random(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        out.extend_from_slice(&z.to_le_bytes());
    }
    out.truncate(len);
    out
}

#[test]
fn pseudo_random_is_stable() {
    // SplitMix64 reference: first output for seed 0 is 0xE220A8397B1DCDAF.
    assert_eq!(
        pseudo_random(0, 8),
        0xE220_A839_7B1D_CDAF_u64.to_le_bytes().to_vec()
    );
}
