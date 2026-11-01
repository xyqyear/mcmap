use super::cache::BlockCache;
use super::{bad, compound, seen, Result};
use crate::{
    biome::Biome, pre18, BiomeData, Block, BlockData, Heightmaps, Section, SectionLike,
    SectionTower,
};
use mcmap_nbt::Cursor;
fn block_palette(c: &mut Cursor<'_>, t: u8, cache: &mut BlockCache) -> Result<Vec<Block>> {
    if t != 9 {
        return Err(bad());
    }
    let (element, n) = c.list_header()?;
    let mut out = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        out.push(cache.parse(c, element)?);
    }
    Ok(out)
}
fn blocks(c: &mut Cursor<'_>, t: u8, cache: &mut BlockCache) -> Result<BlockData<Block>> {
    compound(t)?;
    let (mut mask, mut data, mut palette) = (0, None, None);
    while let Some((t, k)) = c.field()? {
        match k {
            b"data" => {
                seen(&mut mask, 1)?;
                data = Some(c.value(t)?);
            }
            b"palette" => {
                seen(&mut mask, 2)?;
                palette = Some(block_palette(c, t, cache)?);
            }
            _ => c.skip(t)?,
        }
    }
    Ok(BlockData::from_parts(data, palette.ok_or_else(bad)?))
}
fn biomes(c: &mut Cursor<'_>, t: u8) -> Result<BiomeData<Biome>> {
    compound(t)?;
    let (mut mask, mut data, mut palette) = (0, None, None);
    while let Some((t, k)) = c.field()? {
        match k {
            b"data" => {
                seen(&mut mask, 1)?;
                data = Some(c.value(t)?);
            }
            b"palette" => {
                seen(&mut mask, 2)?;
                palette = Some(c.value(t)?);
            }
            _ => c.skip(t)?,
        }
    }
    Ok(BiomeData::from_parts(data, palette.ok_or_else(bad)?))
}
pub(super) fn heightmaps(c: &mut Cursor<'_>, t: u8) -> Result<Heightmaps> {
    compound(t)?;
    let (mut mask, mut motion_blocking) = (0, None);
    while let Some((t, k)) = c.field()? {
        if k == b"MOTION_BLOCKING" {
            seen(&mut mask, 1)?;
            motion_blocking = Some(c.value(t)?);
        } else {
            c.skip(t)?;
        }
    }
    Ok(Heightmaps { motion_blocking })
}
pub(super) fn section(c: &mut Cursor<'_>, t: u8, cache: &mut BlockCache) -> Result<Section> {
    compound(t)?;
    let (mut mask, mut y, mut block_states, mut biomes_value) =
        (0, None, BlockData::default(), BiomeData::default());
    while let Some((t, k)) = c.field()? {
        match k {
            b"Y" => {
                seen(&mut mask, 1)?;
                y = Some(c.value(t)?);
            }
            b"block_states" => {
                seen(&mut mask, 2)?;
                block_states = blocks(c, t, cache)?;
            }
            b"biomes" => {
                seen(&mut mask, 4)?;
                biomes_value = biomes(c, t)?;
            }
            _ => c.skip(t)?,
        }
    }
    Ok(Section {
        y: y.ok_or_else(bad)?,
        block_states,
        biomes: biomes_value,
    })
}
pub(super) fn section18(
    c: &mut Cursor<'_>,
    t: u8,
    cache: &mut BlockCache,
) -> Result<pre18::Pre18Section> {
    compound(t)?;
    let (mut mask, mut y, mut block_states, mut palette) = (0, None, None, Vec::new());
    while let Some((t, k)) = c.field()? {
        match k {
            b"Y" => {
                seen(&mut mask, 1)?;
                y = Some(c.value(t)?);
            }
            b"BlockStates" => {
                seen(&mut mask, 2)?;
                block_states = Some(c.value(t)?);
            }
            b"Palette" => {
                seen(&mut mask, 4)?;
                palette = block_palette(c, t, cache)?;
            }
            _ => c.skip(t)?,
        }
    }
    Ok(pre18::Pre18Section {
        y: y.ok_or_else(bad)?,
        block_states,
        palette,
    })
}
pub(super) fn sections<S: SectionLike>(
    c: &mut Cursor<'_>,
    t: u8,
    cache: &mut BlockCache,
    parse: fn(&mut Cursor<'_>, u8, &mut BlockCache) -> Result<S>,
) -> Result<SectionTower<S>> {
    if t != 9 {
        return Err(bad());
    }
    let (element, n) = c.list_header()?;
    let mut out = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        out.push(parse(c, element, cache)?);
    }
    Ok(SectionTower::from_sections(out))
}
