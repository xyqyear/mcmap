use super::{bad, compound, seen, Result};
use super::{cache::BlockCache, sections::*};
use crate::{pre18, CurrentJavaChunk, JavaChunk};
use mcmap_nbt::Cursor;
use std::sync::RwLock;
fn current(bytes: &[u8], cache: &mut BlockCache) -> Result<JavaChunk> {
    let mut c = Cursor::root(bytes)?;
    let (mut mask, mut data_version, mut status, mut section_data, mut heights) =
        (0, None, None, None, None);
    while let Some((t, k)) = c.field()? {
        match k {
            b"DataVersion" => {
                seen(&mut mask, 1)?;
                data_version = Some(c.value(t)?);
            }
            b"Status" => {
                seen(&mut mask, 2)?;
                status = Some(c.value(t)?);
            }
            b"sections" => {
                seen(&mut mask, 4)?;
                section_data = Some(sections(&mut c, t, cache, section)?);
            }
            b"Heightmaps" => {
                seen(&mut mask, 8)?;
                heights = Some(heightmaps(&mut c, t)?);
            }
            _ => c.skip(t)?,
        }
    }
    Ok(JavaChunk::Post18(CurrentJavaChunk {
        data_version: data_version.ok_or_else(bad)?,
        status: status.ok_or_else(bad)?,
        sections: section_data,
        heightmaps: heights,
        lazy_heightmap: RwLock::new(None),
    }))
}
fn level18(c: &mut Cursor<'_>, t: u8, cache: &mut BlockCache) -> Result<pre18::Level> {
    compound(t)?;
    let (mut mask, mut x_pos, mut z_pos, mut status, mut biome_data, mut section_data, mut heights) =
        (0, None, None, None, None, None, None);
    while let Some((t, k)) = c.field()? {
        match k {
            b"xPos" => {
                seen(&mut mask, 1)?;
                x_pos = Some(c.value(t)?);
            }
            b"zPos" => {
                seen(&mut mask, 2)?;
                z_pos = Some(c.value(t)?);
            }
            b"Status" => {
                seen(&mut mask, 4)?;
                status = Some(c.value(t)?);
            }
            b"Biomes" => {
                seen(&mut mask, 8)?;
                biome_data = Some(c.value(t)?);
            }
            b"Sections" => {
                seen(&mut mask, 16)?;
                section_data = Some(sections(c, t, cache, section18)?);
            }
            b"Heightmaps" => {
                seen(&mut mask, 32)?;
                heights = Some(heightmaps(c, t)?);
            }
            _ => c.skip(t)?,
        }
    }
    Ok(pre18::Level {
        x_pos: x_pos.ok_or_else(bad)?,
        z_pos: z_pos.ok_or_else(bad)?,
        status: status.ok_or_else(bad)?,
        biomes: biome_data,
        sections: section_data,
        heightmaps: heights,
        lazy_heightmap: RwLock::new(None),
    })
}
fn old(bytes: &[u8], cache: &mut BlockCache) -> Result<JavaChunk> {
    let mut c = Cursor::root(bytes)?;
    let (mut mask, mut data_version, mut level) = (0, None, None);
    while let Some((t, k)) = c.field()? {
        match k {
            b"DataVersion" => {
                seen(&mut mask, 1)?;
                data_version = Some(c.value(t)?);
            }
            b"Level" => {
                seen(&mut mask, 2)?;
                level = Some(level18(&mut c, t, cache)?);
            }
            b"Status" => return Err(bad()),
            _ => c.skip(t)?,
        }
    }
    Ok(JavaChunk::Pre18(pre18::JavaChunk {
        data_version: data_version.ok_or_else(bad)?,
        level: level.ok_or_else(bad)?,
    }))
}
pub(super) fn decode(bytes: &[u8], cache: &mut BlockCache) -> Result<JavaChunk> {
    let mut c = Cursor::root(bytes)?;
    while let Some((t, k)) = c.field()? {
        match (t, k) {
            (10, b"Level") => return old(bytes, cache),
            (_, b"Status" | b"sections") => return current(bytes, cache),
            _ => c.skip(t)?,
        }
    }
    current(bytes, cache)
}
