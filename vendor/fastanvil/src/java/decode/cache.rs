use crate::Block;
use ahash::AHashMap;
use mcmap_nbt::{Cursor, Result};

#[cfg(test)]
mod tests;

const MAX_ENTRIES: usize = 4096;
const MAX_KEY_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct BlockCache {
    maps: [AHashMap<Vec<u8>, Block>; 2],
    bytes: usize,
}

impl BlockCache {
    pub(super) fn parse(&mut self, cursor: &mut Cursor<'_>, tag: u8) -> Result<Block> {
        if !matches!(tag, 8 | 10) {
            return cursor.value(tag);
        }
        let start = cursor.position();
        let mut end = cursor.clone();
        end.skip(tag)?;
        let key = cursor.raw_range(start, end.position());
        if key.len() > MAX_ENTRY_BYTES {
            return cursor.value(tag);
        }
        let map = usize::from(tag == 10);
        if let Some(block) = self.maps[map].get(key) {
            *cursor = end;
            return Ok(block.clone());
        }
        // Only validated, fully owned blocks can outlive the input buffer.
        let block: Block = cursor.value(tag)?;
        if self.maps[0].len() + self.maps[1].len() >= MAX_ENTRIES
            || self.bytes + key.len() > MAX_KEY_BYTES
        {
            *self = Self::default();
        }
        self.bytes += key.len();
        self.maps[map].insert(key.to_vec(), block.clone());
        Ok(block)
    }
}
