use bit_field::BitField;
use mcmap_nbt::LongArray;

use serde::Deserialize;
use std::fmt::Debug;

#[derive(Debug)]
pub struct BlockData<T: Debug> {
    inner: DataInner<T>,
    packing: Packing,
}

#[derive(Debug)]
pub struct BiomeData<T: Debug> {
    inner: DataInner<T>,
    packing: Packing,
}

#[derive(Debug, Clone, Copy)]
struct Packing {
    bits: usize,
    per_long: usize,
    reciprocal: u64,
    mask: u64,
}

impl Packing {
    fn new(palette_len: usize, minimum_bits: usize) -> Self {
        let bits = minimum_bits.max(min_bits_for_n_states(palette_len));
        let per_long = 64 / bits;
        Self {
            bits,
            per_long,
            reciprocal: ((1u64 << 32) + per_long as u64 - 1) / per_long as u64,
            mask: u64::MAX >> (64 - bits),
        }
    }

    fn long_index(self, index: usize) -> usize {
        if index < 4096 {
            // For these indices and divisors <= 64, rounding the reciprocal cannot change the quotient.
            ((index as u64 * self.reciprocal) >> 32) as usize
        } else {
            index / self.per_long
        }
    }
}

impl<'de, T: Debug + Deserialize<'de>> Deserialize<'de> for BlockData<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let inner = DataInner::deserialize(deserializer)?;
        Ok(Self::from_parts(inner.data, inner.palette))
    }
}

impl<'de, T: Debug + Deserialize<'de>> Deserialize<'de> for BiomeData<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let inner = DataInner::deserialize(deserializer)?;
        Ok(Self::from_parts(inner.data, inner.palette))
    }
}

impl<T: Debug> BlockData<T> {
    pub(super) fn from_parts(data: Option<LongArray>, palette: Vec<T>) -> Self {
        let packing = Packing::new(palette.len(), 4);
        Self {
            inner: DataInner { data, palette },
            packing,
        }
    }

    /// Get the block data for the block at x,y,z, where x,y,z are relative to
    /// the section ie 0..16.
    pub fn at(&self, x: usize, sec_y: usize, z: usize) -> Option<&T> {
        let state_index = (sec_y * 16 * 16) + z * 16 + x;
        self.inner.at(state_index, self.packing)
    }

    /// Get iterator for the state indicies. This will increase in x, then z,
    /// then y. These indicies are used with the relevant palette to get the
    /// data for that block.
    ///
    /// This returns None if no block states were present. This typically means
    /// the section was empty (ie filled with air).
    ///
    /// You can recover the coordinate be enumerating the iterator:
    ///
    /// ```no_run
    /// # use fastanvil::BlockData;
    /// # let states: BlockData<usize> = todo!();
    /// for (i, block_index) in states.try_iter_indices().unwrap().enumerate() {
    ///     let x = i & 0x000F;
    ///     let y = (i & 0x0F00) >> 8;
    ///     let z = (i & 0x00F0) >> 4;
    /// }
    /// ```
    pub fn try_iter_indices(&self) -> Option<StatesIter<'_>> {
        if let Some(data) = &self.inner.data {
            Some(StatesIter::new(self.packing.bits, 16 * 16 * 16, data))
        } else {
            None
        }
    }

    /// Get the palette for this block data. Indicies into this palette can be
    /// obtained via [`try_iter_indices`][`BlockData::try_iter_indices`].
    pub fn palette(&self) -> &[T] {
        self.inner.palette.as_slice()
    }
}

impl<T: Debug> BiomeData<T> {
    pub(super) fn from_parts(data: Option<LongArray>, palette: Vec<T>) -> Self {
        let packing = Packing::new(palette.len(), 1);
        Self {
            inner: DataInner { data, palette },
            packing,
        }
    }

    pub fn at(&self, x: usize, sec_y: usize, z: usize) -> Option<&T> {
        // Caution: int division, so lops of remainder of 4, so you can't just
        // remove a *4 and /4 and get the same results.
        let x = x / 4;
        let y = sec_y / 4;
        let z = z / 4;

        let state_index = (y * 4 * 4) + z * 4 + x;
        self.inner.at(state_index, self.packing)
    }

    pub fn try_iter_indices(&self) -> Option<StatesIter<'_>> {
        if let Some(data) = &self.inner.data {
            Some(StatesIter::new(self.packing.bits, 4 * 4 * 4, data))
        } else {
            None
        }
    }

    pub fn palette(&self) -> &[T] {
        self.inner.palette.as_slice()
    }
}

#[derive(Deserialize, Debug)]
struct DataInner<T: Debug> {
    data: Option<LongArray>,
    palette: Vec<T>,
}

impl<T: Debug> DataInner<T> {
    fn at(&self, index: usize, packing: Packing) -> Option<&T> {
        if self.data.is_none() && self.palette.len() == 1 {
            return self.palette.get(0);
        }

        let data = self.data.as_ref()?;

        let long_index = packing.long_index(index);
        let shift = (index - long_index * packing.per_long) * packing.bits;

        // Super important line: treat the i64 as an u64.
        // Bug 1: Kept i64 and the get_bits interprets as signed.
        // Bug 2: Went to usize, worked on 64bit platforms broke on 32 bit like WASM.
        let long = data[long_index] as u64;

        let palette_index = (long >> shift) & packing.mask;

        self.palette.get(palette_index as usize)
    }
}

// Block states at the least can be missing from the world data. This typically
// just means that it's a big block of air. We default the DataInner and let the
// fact data is None to also return none. Rather than have BlockData be optional
// in the chunk struct.
impl<T: Debug> Default for DataInner<T> {
    fn default() -> Self {
        Self {
            data: Default::default(),
            palette: Default::default(),
        }
    }
}

impl<T: Debug> Default for BlockData<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
            packing: Packing::new(0, 4),
        }
    }
}

impl<T: Debug> Default for BiomeData<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
            packing: Packing::new(0, 1),
        }
    }
}

pub(crate) fn min_bits_for_n_states(palette_len: usize) -> usize {
    // Empty palettes retain the word-sized packing used by the original release build.
    (usize::BITS - palette_len.wrapping_sub(1).leading_zeros()) as usize
}

/// Iterator over block state data. Each value is the index into the relevant palette.
pub struct StatesIter<'a> {
    inner: &'a [i64], // remaining data to iterate.
    stride: usize,    // stride in bits we're iterating.
    pos: usize,       // bit position the next value starts at.
    // need to track the 'remaining' because some of the last long might be padding.
    remaining: usize,
}

impl<'a> StatesIter<'a> {
    pub(crate) fn new(stride: usize, len: usize, inner: &'a [i64]) -> Self {
        Self {
            inner,
            stride,
            remaining: len,
            pos: 0,
        }
    }
}

impl<'a> Iterator for StatesIter<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let start = self.pos;
        self.pos += self.stride;
        let end = self.pos;
        let datum = *(self.inner.first()?) as u64;

        let value = datum.get_bits(start..end) as usize;
        self.remaining -= 1;

        if self.pos + self.stride > 64 {
            self.pos = 0;
            self.inner = &self.inner[1..];
        }

        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a> ExactSizeIterator for StatesIter<'a> {}

#[cfg(test)]
mod packing_tests {
    use super::{BiomeData, BlockData, Packing};

    #[test]
    fn empty_palettes_keep_word_sized_packed_indices() {
        let nbt = mcmap_nbt::nbt!({
            "palette": [],
            "data": mcmap_nbt::LongArray::new(vec![0, 1, 2, 3]),
        });
        let bytes = mcmap_nbt::to_bytes(&nbt).unwrap();
        let blocks: BlockData<String> = mcmap_nbt::from_bytes(&bytes).unwrap();
        let biomes: BiomeData<String> = mcmap_nbt::from_bytes(&bytes).unwrap();
        assert_eq!(blocks.packing.bits, usize::BITS as usize);
        assert_eq!(biomes.packing.bits, usize::BITS as usize);
        assert_eq!(blocks.at(0, 0, 0), None);
        assert_eq!(biomes.at(0, 0, 0), None);
        if usize::BITS == 64 {
            assert_eq!(
                blocks
                    .try_iter_indices()
                    .unwrap()
                    .take(4)
                    .collect::<Vec<_>>(),
                [0, 1, 2, 3]
            );
            assert_eq!(
                biomes
                    .try_iter_indices()
                    .unwrap()
                    .take(4)
                    .collect::<Vec<_>>(),
                [0, 1, 2, 3]
            );
            assert!(std::panic::catch_unwind(|| blocks.at(4, 0, 0)).is_err());
        }
    }

    #[test]
    fn reciprocal_matches_division_for_all_section_indices() {
        for per_long in 1..=64 {
            let packing = Packing {
                bits: 1,
                per_long,
                reciprocal: ((1u64 << 32) + per_long as u64 - 1) / per_long as u64,
                mask: 1,
            };
            for index in 0..4096 {
                let long_index = packing.long_index(index);
                assert_eq!(long_index, index / per_long);
                assert_eq!(index - long_index * per_long, index % per_long);
            }
        }
    }

    #[test]
    fn reciprocal_keeps_division_for_indices_outside_a_section() {
        for per_long in 1..=64 {
            let packing = Packing {
                bits: 1,
                per_long,
                reciprocal: ((1u64 << 32) + per_long as u64 - 1) / per_long as u64,
                mask: 1,
            };
            for index in [4096, 4097, 65535, 1 << 20, usize::MAX - 1, usize::MAX] {
                let long_index = packing.long_index(index);
                assert_eq!(long_index, index / per_long);
                assert_eq!(index - long_index * per_long, index % per_long);
            }
        }
    }
}
