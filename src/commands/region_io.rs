// Byte-level Anvil region I/O shared by replace-chunks and remove-chunks.
//
// Handles the 1.15+ `.mcc` external-chunk overflow mechanism (scheme byte high
// bit + sibling `c.<absX>.<absZ>.mcc` file) without decoding NBT.
//
// See `docs/replace_chunks.md` for the format spec and atomicity argument.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use log::warn;

use super::util::parse_region_filename;
use crate::chown;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub const SECTOR_BYTES: usize = 4096;
pub const HEADER_SECTORS: usize = 2;
pub const SLOT_COUNT: usize = 1024;
pub const MAX_INLINE_SECTORS: usize = 255;

#[derive(Clone, Debug)]
pub enum SlotState {
    Empty,
    Inline {
        scheme: u8,
        payload: Vec<u8>,
        timestamp: u32,
    },
    /// `mcc` is `Some` only when this slot was loaded from a source for
    /// replacement; preserved-as-is target slots leave it `None` because
    /// the re-emit only needs the stub record.
    External {
        scheme: u8,
        timestamp: u32,
        mcc: Option<Vec<u8>>,
    },
}

pub fn parse_chunks(s: &str) -> std::result::Result<Vec<(u8, u8)>, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("--chunks must contain at least one coord".into());
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in trimmed.split(';') {
        let part = raw.trim();
        if part.is_empty() {
            return Err(format!("empty entry in --chunks: {:?}", s));
        }
        let mut iter = part.split(',');
        let xs = iter.next().unwrap_or("").trim();
        let zs = iter
            .next()
            .ok_or_else(|| format!("missing z in coord {:?}", part))?
            .trim();
        if iter.next().is_some() {
            return Err(format!("invalid coord {:?}: expected x,z", part));
        }
        let x: u8 = xs
            .parse()
            .map_err(|_| format!("invalid x in coord {:?}", part))?;
        let z: u8 = zs
            .parse()
            .map_err(|_| format!("invalid z in coord {:?}", part))?;
        if x >= 32 || z >= 32 {
            return Err(format!("coord {},{} out of range [0, 31]", x, z));
        }
        if !seen.insert((x, z)) {
            return Err(format!("duplicate coord {},{} in --chunks", x, z));
        }
        out.push((x, z));
    }
    Ok(out)
}

pub fn region_coords(path: &Path) -> Option<(isize, isize)> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    let (x, z) = parse_region_filename(name)?;
    Some((x.0, z.0))
}

pub fn slot_index(x: u8, z: u8) -> usize {
    (x as usize) + (z as usize) * 32
}

fn slot_to_xz(slot: usize) -> (u8, u8) {
    ((slot % 32) as u8, (slot / 32) as u8)
}

pub fn read_slot(
    bytes: &[u8],
    slot: usize,
    dir: &Path,
    region: Option<(isize, isize)>,
    load_mcc: bool,
    side: &str,
) -> Result<SlotState> {
    let loc_off = slot * 4;
    let loc = &bytes[loc_off..loc_off + 4];
    let sector_offset =
        ((loc[0] as u32) << 16) | ((loc[1] as u32) << 8) | (loc[2] as u32);
    let sector_count = loc[3];
    if sector_offset == 0 || sector_count == 0 {
        return Ok(SlotState::Empty);
    }

    // Vanilla's RegionFile constructor (1.16+) clears any slot whose offset
    // points into the header sectors, into oblivion past EOF, or whose count
    // is zero. Mirror that by treating those cases as empty rather than
    // erroring out — a single corrupt slot shouldn't poison the whole
    // operation. For older vanilla (1.7-1.15) the same outcome happens
    // lazily on read (returns null). Either way the user-observable answer
    // is "no chunk here", and matching that lets mcmap operate on
    // partially-corrupt regions instead of failing closed.
    let (x, z) = slot_to_xz(slot);
    if sector_offset < HEADER_SECTORS as u32 {
        warn!(
            "{}: chunk at ({}, {}) overlaps with region header (sector_offset={}); treating as empty",
            side, x, z, sector_offset
        );
        return Ok(SlotState::Empty);
    }

    let ts_off = SECTOR_BYTES + slot * 4;
    let timestamp =
        u32::from_be_bytes(bytes[ts_off..ts_off + 4].try_into().unwrap());

    let record_off = (sector_offset as usize) * SECTOR_BYTES;
    if record_off + 5 > bytes.len() {
        warn!(
            "{}: chunk at ({}, {}) header extends past end of file (sector_offset={}, file_len={}); treating as empty",
            side, x, z, sector_offset, bytes.len()
        );
        return Ok(SlotState::Empty);
    }
    let length =
        u32::from_be_bytes(bytes[record_off..record_off + 4].try_into().unwrap());
    let scheme = bytes[record_off + 4];
    if length < 1 {
        warn!(
            "{}: chunk at ({}, {}) has invalid length 0; treating as empty",
            side, x, z
        );
        return Ok(SlotState::Empty);
    }
    // Vanilla rejects records whose claimed length exceeds the slot's
    // sector reservation (`length > sector_count * 4096` in RegionFile).
    // The on-disk record is `4 (length field) + length` bytes, and
    // `length` includes the scheme byte; vanilla's check uses the looser
    // `length > 4096*count` bound, which we mirror exactly.
    let max_length = (sector_count as u32) * (SECTOR_BYTES as u32);
    if length > max_length {
        warn!(
            "{}: chunk at ({}, {}) claims length {} but sector reservation is {} bytes; treating as empty",
            side, x, z, length, max_length
        );
        return Ok(SlotState::Empty);
    }

    if scheme & 0x80 != 0 {
        let mcc = if load_mcc {
            let (rx, rz) = region.ok_or_else(|| {
                format!(
                    "{} filename is not r.X.Z.mca: cannot derive .mcc filename for external chunk at slot {}",
                    side, slot
                )
            })?;
            let x_rel = (slot as isize) & 31;
            let z_rel = ((slot as isize) >> 5) & 31;
            let abs_x = rx * 32 + x_rel;
            let abs_z = rz * 32 + z_rel;
            let path = dir.join(format!("c.{}.{}.mcc", abs_x, abs_z));
            let bytes = std::fs::read(&path).map_err(|e| {
                format!(
                    "{}: failed to read external chunk file {}: {}",
                    side,
                    path.display(),
                    e
                )
            })?;
            Some(bytes)
        } else {
            None
        };
        Ok(SlotState::External {
            scheme,
            timestamp,
            mcc,
        })
    } else {
        let payload_len = (length - 1) as usize;
        let start = record_off + 5;
        let end = start + payload_len;
        if end > bytes.len() {
            warn!(
                "{}: chunk at ({}, {}) payload extends past end of file (claimed {} payload bytes, file_len={}); treating as empty",
                side, x, z, payload_len, bytes.len()
            );
            return Ok(SlotState::Empty);
        }
        Ok(SlotState::Inline {
            scheme,
            payload: bytes[start..end].to_vec(),
            timestamp,
        })
    }
}

/// Vanilla writes a 0-byte `r.X.Z.mca` whenever it has nothing to record for
/// that region — most commonly under `poi/`, but the same rule covers `region/`
/// and `entities/`. `RegionFile.<init>` in 1.21.x reads the header, sees
/// `FileChannel.read(...) == -1`, skips the location-table parse loop, and
/// returns a region with all 1024 slots unallocated. Treat the same on disk
/// shape as "no chunks here" instead of an error so `replace-chunks` and
/// `remove-chunks` keep parity with vanilla on real worlds.
pub fn is_placeholder_region(bytes: &[u8]) -> bool {
    bytes.len() < HEADER_SECTORS * SECTOR_BYTES
}

fn write_location(buf: &mut [u8], slot: usize, sector_offset: u32, sector_count: u8) {
    let off = slot * 4;
    buf[off] = ((sector_offset >> 16) & 0xFF) as u8;
    buf[off + 1] = ((sector_offset >> 8) & 0xFF) as u8;
    buf[off + 2] = (sector_offset & 0xFF) as u8;
    buf[off + 3] = sector_count;
}

fn write_timestamp(buf: &mut [u8], slot: usize, timestamp: u32) {
    let off = SECTOR_BYTES + slot * 4;
    buf[off..off + 4].copy_from_slice(&timestamp.to_be_bytes());
}

pub fn emit_region(slots: &[SlotState]) -> Result<Vec<u8>> {
    let mut out = vec![0u8; HEADER_SECTORS * SECTOR_BYTES];
    let mut current_sector: u32 = HEADER_SECTORS as u32;

    for (slot, state) in slots.iter().enumerate() {
        match state {
            SlotState::Empty => {}
            SlotState::Inline {
                scheme,
                payload,
                timestamp,
            } => {
                let total = 5 + payload.len();
                let sectors = total.div_ceil(SECTOR_BYTES);
                if sectors > MAX_INLINE_SECTORS {
                    return Err(format!(
                        "chunk at slot {} requires {} sectors (>{}); refusing to emit a corrupt record",
                        slot, sectors, MAX_INLINE_SECTORS
                    )
                    .into());
                }
                let length: u32 = (1 + payload.len()) as u32;
                out.extend_from_slice(&length.to_be_bytes());
                out.push(*scheme);
                out.extend_from_slice(payload);
                let pad = sectors * SECTOR_BYTES - total;
                out.resize(out.len() + pad, 0);

                write_location(&mut out, slot, current_sector, sectors as u8);
                write_timestamp(&mut out, slot, *timestamp);
                current_sector += sectors as u32;
            }
            SlotState::External {
                scheme, timestamp, ..
            } => {
                out.extend_from_slice(&1u32.to_be_bytes());
                out.push(*scheme);
                out.resize(out.len() + SECTOR_BYTES - 5, 0);

                write_location(&mut out, slot, current_sector, 1);
                write_timestamp(&mut out, slot, *timestamp);
                current_sector += 1;
            }
        }
    }

    Ok(out)
}

fn write_atomic(tmp: &Path, dst: &Path, bytes: &[u8]) -> Result<()> {
    {
        let mut f = File::create(tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, dst)?;
    chown::apply(dst)?;
    Ok(())
}

/// Apply per-slot mutations to a target region file. For each `(slot,
/// new_state)`, the target's slot is overwritten; slots not listed are
/// preserved verbatim. A missing or sub-header target file is treated as a
/// 1024-empty-slot region (matching vanilla — see `is_placeholder_region`).
/// If the target was such a placeholder and no chunk ended up present after
/// the mutations, the file is left untouched: vanilla's 0-byte poi/entities
/// placeholders stay 0-byte instead of being promoted to an 8192-byte
/// all-zero header for no reason.
///
/// External slots in `new_state` must carry their `.mcc` bytes inline (i.e.
/// `mcc: Some(_)`); the bytes are written next to the target as
/// `c.<absX>.<absZ>.mcc` using the target's region coords.
///
/// Atomicity ordering — see `docs/replace_chunks.md` §12. At no point may a
/// stub record exist on disk without its companion `.mcc` file:
///   1. Write all new/replacement `.mcc` files (tmp + fsync + rename).
///   2. Atomic-rename the new target `.mca` over the old one.
///   3. Delete stale `.mcc` files.
pub fn apply_slot_mutations(
    target: &Path,
    mutations: &[(usize, SlotState)],
) -> Result<()> {
    let target_region = region_coords(target);
    let target_dir = target.parent().unwrap_or(Path::new(""));

    let (mut target_slots, target_was_placeholder) = match std::fs::read(target) {
        Ok(ref b) if !is_placeholder_region(b) => {
            let mut v = Vec::with_capacity(SLOT_COUNT);
            for slot in 0..SLOT_COUNT {
                v.push(read_slot(b, slot, target_dir, target_region, false, "target")?);
            }
            (v, false)
        }
        Ok(_) => (vec![SlotState::Empty; SLOT_COUNT], true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (vec![SlotState::Empty; SLOT_COUNT], true)
        }
        Err(e) => return Err(e.into()),
    };

    let prior_external: Vec<bool> = mutations
        .iter()
        .map(|(slot, _)| matches!(target_slots[*slot], SlotState::External { .. }))
        .collect();

    for (slot, new_state) in mutations {
        target_slots[*slot] = new_state.clone();
    }

    let mut mcc_writes: Vec<(isize, isize, Vec<u8>)> = Vec::new();
    let mut mcc_deletes: Vec<(isize, isize)> = Vec::new();

    for (i, (slot, _)) in mutations.iter().enumerate() {
        let new_external = matches!(target_slots[*slot], SlotState::External { .. });
        if !new_external && !prior_external[i] {
            continue;
        }
        let (rx, rz) = target_region.ok_or_else(|| -> Box<dyn std::error::Error> {
            let (x, z) = slot_to_xz(*slot);
            format!(
                "target filename is not r.X.Z.mca: cannot derive .mcc filename for slot ({}, {})",
                x, z
            )
            .into()
        })?;
        let x_rel = (*slot as isize) & 31;
        let z_rel = ((*slot as isize) >> 5) & 31;
        let abs_x = rx * 32 + x_rel;
        let abs_z = rz * 32 + z_rel;
        match &target_slots[*slot] {
            SlotState::External { mcc, .. } => {
                let bytes = mcc.clone().ok_or_else(|| -> Box<dyn std::error::Error> {
                    "internal error: replaced external slot has no .mcc bytes".into()
                })?;
                mcc_writes.push((abs_x, abs_z, bytes));
            }
            _ => {
                mcc_deletes.push((abs_x, abs_z));
            }
        }
    }

    if target_was_placeholder
        && target_slots.iter().all(|s| matches!(s, SlotState::Empty))
    {
        // Nothing to materialize. Leaving a 0-byte vanilla placeholder alone
        // (or never creating a missing target) is the right outcome — see
        // `is_placeholder_region` for the parity argument with vanilla. The
        // .mcc bookkeeping above produces no work in this case because a
        // placeholder target carried no prior external slots.
        debug_assert!(mcc_writes.is_empty() && mcc_deletes.is_empty());
        return Ok(());
    }

    let out_bytes = emit_region(&target_slots)?;

    for (ax, az, bytes) in &mcc_writes {
        let final_path = target_dir.join(format!("c.{}.{}.mcc", ax, az));
        let tmp_path = target_dir.join(format!("c.{}.{}.mcc.tmp", ax, az));
        write_atomic(&tmp_path, &final_path, bytes)?;
    }

    let target_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| -> Box<dyn std::error::Error> {
            "target has no file name component".into()
        })?;
    let tmp_target = target_dir.join(format!("{}.tmp", target_name));
    write_atomic(&tmp_target, target, &out_bytes)?;

    for (ax, az) in &mcc_deletes {
        let path = target_dir.join(format!("c.{}.{}.mcc", ax, az));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "failed to delete stale .mcc {}: {}",
                    path.display(),
                    e
                )
                .into());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chunks_basic() {
        assert_eq!(parse_chunks("4,15").unwrap(), vec![(4, 15)]);
        assert_eq!(
            parse_chunks("4,15;4,14;13,22").unwrap(),
            vec![(4, 15), (4, 14), (13, 22)]
        );
        assert_eq!(
            parse_chunks(" 0 , 0 ; 31, 31 ").unwrap(),
            vec![(0, 0), (31, 31)]
        );
    }

    #[test]
    fn parse_chunks_rejects_empty() {
        assert!(parse_chunks("").is_err());
        assert!(parse_chunks("   ").is_err());
    }

    #[test]
    fn parse_chunks_rejects_out_of_range() {
        assert!(parse_chunks("32,0").is_err());
        assert!(parse_chunks("0,32").is_err());
    }

    #[test]
    fn parse_chunks_rejects_duplicates() {
        assert!(parse_chunks("4,15;4,15").is_err());
    }

    #[test]
    fn parse_chunks_rejects_malformed() {
        assert!(parse_chunks("4").is_err());
        assert!(parse_chunks("4,").is_err());
        assert!(parse_chunks(",4").is_err());
        assert!(parse_chunks("4,5,6").is_err());
        assert!(parse_chunks("4,15;").is_err());
        assert!(parse_chunks("a,b").is_err());
    }

    #[test]
    fn placeholder_region_threshold() {
        assert!(is_placeholder_region(b""));
        assert!(is_placeholder_region(&[0u8; 1]));
        assert!(is_placeholder_region(
            &[0u8; HEADER_SECTORS * SECTOR_BYTES - 1]
        ));
        assert!(!is_placeholder_region(
            &[0u8; HEADER_SECTORS * SECTOR_BYTES]
        ));
        assert!(!is_placeholder_region(
            &[0u8; HEADER_SECTORS * SECTOR_BYTES + 4096]
        ));
    }

    /// Wraps a unique tempdir under the OS temp root. Cleans itself up on drop
    /// so tests don't leak state across runs.
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let p = std::env::temp_dir().join(format!("mcmap-test-{label}-{pid}-{n}"));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Build a minimal valid 8192-byte all-zero region (header only, no
    /// chunks). Equivalent to what vanilla writes when it pads on close with
    /// no chunks present, and what the bug report calls a "header-only" MCA.
    fn empty_header_only() -> Vec<u8> {
        vec![0u8; HEADER_SECTORS * SECTOR_BYTES]
    }

    /// Build a region with one inline chunk record at `(x, z)`. Header is
    /// computed; payload is `payload` (a few bytes of compressed data —
    /// content is opaque to the byte-copy path).
    fn region_with_one_chunk(x: u8, z: u8, scheme: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = empty_header_only();
        let slot = slot_index(x, z);
        let sector_offset: u32 = HEADER_SECTORS as u32;
        let total = 5 + payload.len();
        let sectors = total.div_ceil(SECTOR_BYTES);
        let length = (1 + payload.len()) as u32;
        out.extend_from_slice(&length.to_be_bytes());
        out.push(scheme);
        out.extend_from_slice(payload);
        out.resize(out.len() + sectors * SECTOR_BYTES - total, 0);
        // Location entry
        let loc_off = slot * 4;
        out[loc_off] = ((sector_offset >> 16) & 0xFF) as u8;
        out[loc_off + 1] = ((sector_offset >> 8) & 0xFF) as u8;
        out[loc_off + 2] = (sector_offset & 0xFF) as u8;
        out[loc_off + 3] = sectors as u8;
        // Timestamp = 1
        let ts_off = SECTOR_BYTES + slot * 4;
        out[ts_off..ts_off + 4].copy_from_slice(&1u32.to_be_bytes());
        out
    }

    #[test]
    fn placeholder_target_with_only_empty_mutations_leaves_file_alone() {
        // Bug repro Case 3: vanilla writes a 0-byte poi/r.X.Z.mca placeholder.
        // remove-chunks-style mutations (every new state is Empty) should not
        // promote the placeholder to a fresh 8192-byte all-zero header.
        let dir = TempDir::new("placeholder-noop");
        let target = dir.path().join("r.-1.0.mca");
        std::fs::write(&target, b"").unwrap();

        let mutations = vec![
            (slot_index(0, 0), SlotState::Empty),
            (slot_index(5, 7), SlotState::Empty),
        ];
        apply_slot_mutations(&target, &mutations).unwrap();

        let after = std::fs::metadata(&target).unwrap().len();
        assert_eq!(
            after, 0,
            "0-byte placeholder must remain 0 bytes when mutations clear nothing real"
        );
    }

    #[test]
    fn placeholder_target_promotes_when_chunks_arrive() {
        // Bug report's replace-chunks Case 2: source has chunks, target is the
        // 0-byte poi placeholder. Target must be rewritten as a valid region
        // file containing those chunks.
        let dir = TempDir::new("placeholder-promote");
        let target = dir.path().join("r.0.0.mca");
        std::fs::write(&target, b"").unwrap();

        let payload = vec![0xAAu8; 32];
        let mutations = vec![(
            slot_index(3, 4),
            SlotState::Inline {
                scheme: 2,
                payload: payload.clone(),
                timestamp: 42,
            },
        )];
        apply_slot_mutations(&target, &mutations).unwrap();

        let bytes = std::fs::read(&target).unwrap();
        assert!(
            bytes.len() >= HEADER_SECTORS * SECTOR_BYTES + SECTOR_BYTES,
            "target must be promoted to header + at least one chunk sector, got {}",
            bytes.len()
        );
        let recovered = read_slot(
            &bytes,
            slot_index(3, 4),
            dir.path(),
            region_coords(&target),
            false,
            "target",
        )
        .unwrap();
        match recovered {
            SlotState::Inline {
                scheme,
                payload: p,
                timestamp,
            } => {
                assert_eq!(scheme, 2);
                assert_eq!(p, payload);
                assert_eq!(timestamp, 42);
            }
            other => panic!("expected inline slot, got {:?}", other),
        }
    }

    #[test]
    fn missing_target_with_only_empty_mutations_does_not_create_file() {
        // Symmetric to placeholder-target: a missing target should not be
        // materialized when no chunks end up present.
        let dir = TempDir::new("missing-noop");
        let target = dir.path().join("r.0.0.mca");
        let mutations = vec![(slot_index(0, 0), SlotState::Empty)];
        apply_slot_mutations(&target, &mutations).unwrap();
        assert!(
            !target.exists(),
            "missing target with all-empty mutations must not be created"
        );
    }

    #[test]
    fn placeholder_target_with_inline_source_keeps_existing_chunks_intact() {
        // Belt-and-braces: the promote path must not touch sibling .mcc state.
        // A placeholder target carries no priors; there is nothing to delete.
        // Verify no orphan files appear under target_dir.
        let dir = TempDir::new("placeholder-clean");
        let target = dir.path().join("r.0.0.mca");
        std::fs::write(&target, b"").unwrap();

        let mutations = vec![(
            slot_index(1, 2),
            SlotState::Inline {
                scheme: 2,
                payload: vec![0xCCu8; 16],
                timestamp: 7,
            },
        )];
        apply_slot_mutations(&target, &mutations).unwrap();

        // Only the .mca should exist; no stray .mcc
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["r.0.0.mca".to_string()], "got {:?}", names);
    }

    #[test]
    fn populated_target_with_empty_mutations_clears_slot() {
        // Regression: the "skip write" optimization must only fire on
        // placeholder targets. A real target with one chunk must still be
        // rewritten when that chunk is removed.
        let dir = TempDir::new("populated-clear");
        let target = dir.path().join("r.0.0.mca");
        let bytes = region_with_one_chunk(3, 4, 2, &[0xBBu8; 16]);
        std::fs::write(&target, &bytes).unwrap();

        let mutations = vec![(slot_index(3, 4), SlotState::Empty)];
        apply_slot_mutations(&target, &mutations).unwrap();

        let after = std::fs::read(&target).unwrap();
        assert_eq!(
            after.len(),
            HEADER_SECTORS * SECTOR_BYTES,
            "after clearing the only chunk, target must shrink to header-only"
        );
        // All slot entries zero
        assert!(after[..SECTOR_BYTES].iter().all(|&b| b == 0));
    }

    /// Build an 8192-byte all-zero region header, then patch slot `(x, z)` to
    /// point to `(sector_offset, sector_count)`. No chunk record is written —
    /// callers add one (or skip, to deliberately make it OOB).
    fn header_with_slot(x: u8, z: u8, sector_offset: u32, sector_count: u8) -> Vec<u8> {
        let mut out = vec![0u8; HEADER_SECTORS * SECTOR_BYTES];
        let slot = slot_index(x, z);
        let off = slot * 4;
        out[off] = ((sector_offset >> 16) & 0xFF) as u8;
        out[off + 1] = ((sector_offset >> 8) & 0xFF) as u8;
        out[off + 2] = (sector_offset & 0xFF) as u8;
        out[off + 3] = sector_count;
        out
    }

    #[test]
    fn read_slot_treats_header_overlap_as_empty() {
        // Vanilla (1.16+) clears any slot whose offset points into the
        // header sectors. Mirror that: sector_offset = 1 (i.e., into the
        // timestamp table) must be treated as empty, never read as a chunk.
        let bytes = header_with_slot(0, 0, 1, 1);
        let state = read_slot(&bytes, slot_index(0, 0), Path::new(""), None, false, "test").unwrap();
        assert!(matches!(state, SlotState::Empty), "got {:?}", state);
    }

    #[test]
    fn read_slot_treats_header_past_eof_as_empty() {
        // Header points to a sector past EOF. mcmap previously errored out;
        // vanilla returns null. Match vanilla.
        let bytes = header_with_slot(5, 7, 99, 1);
        let state = read_slot(&bytes, slot_index(5, 7), Path::new(""), None, false, "test").unwrap();
        assert!(matches!(state, SlotState::Empty), "got {:?}", state);
    }

    #[test]
    fn read_slot_treats_zero_length_as_empty() {
        // Slot points at sector 2; sector 2 contains a length-0 record, which
        // is invalid (length must include at least the scheme byte).
        let mut bytes = header_with_slot(1, 1, 2, 1);
        bytes.resize(3 * SECTOR_BYTES, 0); // 4096 bytes of chunk data, all zeros
        // length field at bytes[8192..8196] is already 0 -> invalid
        let state = read_slot(&bytes, slot_index(1, 1), Path::new(""), None, false, "test").unwrap();
        assert!(matches!(state, SlotState::Empty), "got {:?}", state);
    }

    #[test]
    fn read_slot_treats_oversized_length_as_empty() {
        // Vanilla rejects records where the claimed length exceeds the slot's
        // sector reservation. Build a slot with sector_count=1 (so max_length
        // = 4096) but encode length=99999 in the chunk record.
        let mut bytes = header_with_slot(2, 3, 2, 1);
        bytes.resize(3 * SECTOR_BYTES, 0);
        // length = 99999 (way > 4096)
        bytes[2 * SECTOR_BYTES..2 * SECTOR_BYTES + 4]
            .copy_from_slice(&99999u32.to_be_bytes());
        bytes[2 * SECTOR_BYTES + 4] = 2; // scheme = zlib
        let state = read_slot(&bytes, slot_index(2, 3), Path::new(""), None, false, "test").unwrap();
        assert!(matches!(state, SlotState::Empty), "got {:?}", state);
    }

    #[test]
    fn read_slot_treats_payload_past_eof_as_empty() {
        // Slot points at sector 2 with sector_count=2 (so max_length=8192),
        // length is within the reservation, but the file is truncated mid-
        // payload — only one sector of payload data is on disk. Match
        // vanilla's "stream truncated -> null" behavior.
        let mut bytes = header_with_slot(4, 5, 2, 2);
        // length = 5000 bytes (claims to extend into the second payload
        // sector that we deliberately won't write); within the 8192-byte
        // reservation, so the length-field check passes.
        bytes.resize(2 * SECTOR_BYTES + 4096, 0); // only one payload sector on disk
        bytes[2 * SECTOR_BYTES..2 * SECTOR_BYTES + 4]
            .copy_from_slice(&5000u32.to_be_bytes());
        bytes[2 * SECTOR_BYTES + 4] = 2;
        let state = read_slot(&bytes, slot_index(4, 5), Path::new(""), None, false, "test").unwrap();
        assert!(matches!(state, SlotState::Empty), "got {:?}", state);
    }

    #[test]
    fn read_slot_corrupt_target_is_recoverable_via_apply() {
        // End-to-end: apply_slot_mutations against a target with a corrupt
        // slot in a position the user is NOT mutating must succeed (the
        // corrupt slot is silently dropped — same effect as vanilla zeroing
        // it on read). The mutation slot is applied normally.
        let dir = TempDir::new("corrupt-target");
        let target = dir.path().join("r.0.0.mca");
        // Build: corrupt slot at (10, 10) (sector_offset past EOF), good
        // empty rest. Then mutate slot (3, 4) with a normal inline chunk.
        let bytes = header_with_slot(10, 10, 999, 1);
        // No chunk data — sector 999 is far past EOF
        std::fs::write(&target, &bytes).unwrap();

        let mutations = vec![(
            slot_index(3, 4),
            SlotState::Inline {
                scheme: 2,
                payload: vec![0xDDu8; 16],
                timestamp: 99,
            },
        )];
        apply_slot_mutations(&target, &mutations).unwrap();

        // After apply, the corrupt slot is gone (zeroed in re-emit) and the
        // mutation slot is present.
        let after = std::fs::read(&target).unwrap();
        let corrupt =
            read_slot(&after, slot_index(10, 10), dir.path(), region_coords(&target), false, "after")
                .unwrap();
        assert!(matches!(corrupt, SlotState::Empty));
        let mutated =
            read_slot(&after, slot_index(3, 4), dir.path(), region_coords(&target), false, "after")
                .unwrap();
        assert!(matches!(mutated, SlotState::Inline { .. }));
    }
}
