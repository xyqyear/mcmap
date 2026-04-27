// Byte-level Anvil region I/O shared by replace-chunks and remove-chunks.
//
// Handles the 1.17+ `.mcc` external-chunk overflow mechanism (scheme byte high
// bit + sibling `c.<absX>.<absZ>.mcc` file) without decoding NBT.
//
// See `docs/replace_chunks.md` for the format spec and atomicity argument.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::Path;

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

    let ts_off = SECTOR_BYTES + slot * 4;
    let timestamp =
        u32::from_be_bytes(bytes[ts_off..ts_off + 4].try_into().unwrap());

    let record_off = (sector_offset as usize) * SECTOR_BYTES;
    if record_off + 5 > bytes.len() {
        return Err(format!(
            "{}: chunk record at slot {} extends past end of file",
            side, slot
        )
        .into());
    }
    let length =
        u32::from_be_bytes(bytes[record_off..record_off + 4].try_into().unwrap());
    let scheme = bytes[record_off + 4];
    if length < 1 {
        return Err(format!(
            "{}: chunk record at slot {} has invalid length 0",
            side, slot
        )
        .into());
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
            return Err(format!(
                "{}: chunk payload at slot {} extends past end of file",
                side, slot
            )
            .into());
        }
        Ok(SlotState::Inline {
            scheme,
            payload: bytes[start..end].to_vec(),
            timestamp,
        })
    }
}

pub fn validate_region_bytes(bytes: &[u8], side: &str) -> Result<()> {
    if bytes.len() % SECTOR_BYTES != 0 {
        return Err(format!(
            "{}: file size {} is not a multiple of {}",
            side,
            bytes.len(),
            SECTOR_BYTES
        )
        .into());
    }
    if bytes.len() < HEADER_SECTORS * SECTOR_BYTES {
        return Err(format!(
            "{}: file is shorter than the {} byte region header",
            side,
            HEADER_SECTORS * SECTOR_BYTES
        )
        .into());
    }
    Ok(())
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
/// preserved verbatim. A missing target file is treated as a 1024-empty-slot
/// region (so callers can write a fresh region by listing all the slots they
/// want present).
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

    let target_bytes_opt = match std::fs::read(target) {
        Ok(b) => {
            validate_region_bytes(&b, "target")?;
            Some(b)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };

    let mut target_slots: Vec<SlotState> = if let Some(ref tb) = target_bytes_opt {
        let mut v = Vec::with_capacity(SLOT_COUNT);
        for slot in 0..SLOT_COUNT {
            v.push(read_slot(tb, slot, target_dir, target_region, false, "target")?);
        }
        v
    } else {
        vec![SlotState::Empty; SLOT_COUNT]
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
}
