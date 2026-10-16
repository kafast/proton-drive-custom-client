//! Block cache for files in the virtual drive.
//!
//! A file is split into blocks of fixed size. [`BlockMap`] remembers
//! which blocks are present locally. That way reading only has to fetch what
//! is missing, and large files are usable right away.

/// Size of a block: 4 MiB. Large enough for few requests to Proton,
/// small enough that short reads (e.g. thumbnails) are fast.
pub const BLOCK_BYTES: u64 = 4 * 1024 * 1024;

/// A contiguous range of a file in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// Start in bytes from the beginning of the file.
    pub offset: u64,
    /// Length in bytes.
    pub length: u64,
}

impl ByteRange {
    /// First byte after the range.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.offset.saturating_add(self.length)
    }
}

/// Which blocks of a file are present locally.
///
/// A block counts as present when its whole content (at the end of the file: up
/// to the end) is available locally, whether downloaded or written locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockMap {
    size: u64,
    /// One bit per block, least significant bit first.
    bits: Vec<u8>,
}

/// Number of blocks for `size` bytes.
fn block_count(size: u64) -> u64 {
    size.div_ceil(BLOCK_BYTES)
}

/// Number of bytes for `blocks` bits.
fn bitmap_len(blocks: u64) -> usize {
    // For realistic file sizes (below 2^64 bytes / 4 MiB / 8) this always fits
    // into `usize`; otherwise the bitmap stays empty and everything counts as missing.
    usize::try_from(blocks.div_ceil(8)).unwrap_or(0)
}

impl BlockMap {
    /// Nothing is present (freshly created placeholder).
    #[must_use]
    pub fn empty(size: u64) -> Self {
        Self {
            size,
            bits: vec![0; bitmap_len(block_count(size))],
        }
    }

    /// Everything is present (e.g. a file newly created locally).
    #[must_use]
    pub fn complete(size: u64) -> Self {
        let mut map = Self::empty(size);
        map.mark_present(ByteRange {
            offset: 0,
            length: size,
        });
        map
    }

    /// Restores a stored bitmap. If the length does not match the
    /// size (damaged entry), everything counts as missing to be safe.
    #[must_use]
    pub fn from_bytes(size: u64, bits: Vec<u8>) -> Self {
        if bits.len() == bitmap_len(block_count(size)) {
            let mut map = Self { size, bits };
            map.clear_unused_bits();
            map
        } else {
            Self::empty(size)
        }
    }

    /// The bitmap for storing.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    /// File size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// `true` if all blocks are present.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        (0..block_count(self.size)).all(|block| self.has(block))
    }

    /// How many bytes are present locally (space used in the cache).
    #[must_use]
    pub fn present_bytes(&self) -> u64 {
        (0..block_count(self.size))
            .filter(|&block| self.has(block))
            .map(|block| self.block_range(block).length)
            .sum()
    }

    /// The missing parts of `wanted`, extended to whole blocks with
    /// adjacent blocks merged; clipped at the end of the file.
    #[must_use]
    pub fn missing(&self, wanted: ByteRange) -> Vec<ByteRange> {
        let end = wanted.end().min(self.size);
        if wanted.length == 0 || wanted.offset >= end {
            return Vec::new();
        }
        let first = wanted.offset / BLOCK_BYTES;
        let last = (end - 1) / BLOCK_BYTES;
        let mut ranges: Vec<ByteRange> = Vec::new();
        for block in (first..=last).filter(|&block| !self.has(block)) {
            let range = self.block_range(block);
            match ranges.last_mut() {
                Some(previous) if previous.end() == range.offset => {
                    previous.length += range.length;
                }
                _ => ranges.push(range),
            }
        }
        ranges
    }

    /// Marks all blocks fully covered by `range` as present.
    /// The last block counts as covered if `range` reaches the end of the file.
    pub fn mark_present(&mut self, range: ByteRange) {
        let end = range.end().min(self.size);
        if range.offset >= end {
            return;
        }
        let first = range.offset.div_ceil(BLOCK_BYTES);
        let last_exclusive = if end == self.size {
            block_count(self.size)
        } else {
            end / BLOCK_BYTES
        };
        for block in first..last_exclusive {
            self.set(block, true);
        }
    }

    /// Changes the file size (truncating, or extending by writing).
    ///
    /// When extending, the new part is present locally (it was created locally).
    /// The previous last block only stays present if it already was.
    pub fn resize(&mut self, new_size: u64) {
        let old_size = self.size;
        let old_last_present = old_size > 0 && self.has((old_size - 1) / BLOCK_BYTES);
        let mut resized = Self::empty(new_size);
        for block in 0..block_count(new_size.min(old_size)) {
            if self.has(block) {
                resized.set(block, true);
            }
        }
        if new_size > old_size {
            let tail_start = if old_size.is_multiple_of(BLOCK_BYTES) || old_last_present {
                old_size
            } else {
                // The partial old block is still partly missing.
                old_size.next_multiple_of(BLOCK_BYTES)
            };
            resized.mark_present(ByteRange {
                offset: tail_start,
                length: new_size.saturating_sub(tail_start),
            });
        }
        *self = resized;
    }

    fn block_range(&self, block: u64) -> ByteRange {
        let offset = block * BLOCK_BYTES;
        ByteRange {
            offset,
            length: BLOCK_BYTES.min(self.size - offset),
        }
    }

    fn position(block: u64) -> Option<(usize, u8)> {
        let byte = usize::try_from(block / 8).ok()?;
        // `block % 8` is always less than 8, so the bit fits into a byte.
        let bit = 1u8 << (block % 8);
        Some((byte, bit))
    }

    fn has(&self, block: u64) -> bool {
        Self::position(block)
            .and_then(|(byte, bit)| self.bits.get(byte).map(|value| value & bit != 0))
            .unwrap_or(false)
    }

    fn set(&mut self, block: u64, present: bool) {
        if let Some((byte, bit)) = Self::position(block)
            && let Some(value) = self.bits.get_mut(byte)
        {
            if present {
                *value |= bit;
            } else {
                *value &= !bit;
            }
        }
    }

    /// Clear the bits after the last block so that equal contents compare equal.
    fn clear_unused_bits(&mut self) {
        let blocks = block_count(self.size);
        let total_bits = u64::try_from(self.bits.len()).unwrap_or(0) * 8;
        for block in blocks..total_bits {
            self.set(block, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MIB: u64 = 1024 * 1024;

    fn range(offset: u64, length: u64) -> ByteRange {
        ByteRange { offset, length }
    }

    #[test]
    fn empty_file_needs_nothing_and_is_complete() {
        let map = BlockMap::empty(0);
        assert!(map.is_complete());
        assert_eq!(map.missing(range(0, 10)), vec![]);
        assert_eq!(map.present_bytes(), 0);
    }

    #[test]
    fn missing_blocks_are_aligned_merged_and_clipped() {
        let size = 3 * BLOCK_BYTES + MIB;
        let mut map = BlockMap::empty(size);
        assert_eq!(map.missing(range(5, 10)), vec![range(0, BLOCK_BYTES)]);
        assert_eq!(map.missing(range(0, size + 100)), vec![range(0, size)]);
        map.mark_present(range(BLOCK_BYTES, BLOCK_BYTES));
        assert_eq!(
            map.missing(range(0, size)),
            vec![
                range(0, BLOCK_BYTES),
                range(2 * BLOCK_BYTES, BLOCK_BYTES + MIB)
            ]
        );
        assert_eq!(map.missing(range(size, 10)), vec![], "past the end");
        assert_eq!(map.missing(range(0, 0)), vec![]);
    }

    #[test]
    fn only_fully_covered_blocks_count_as_present() {
        let size = 2 * BLOCK_BYTES + 10;
        let mut map = BlockMap::empty(size);
        map.mark_present(range(1, BLOCK_BYTES));
        assert_eq!(map.present_bytes(), 0, "block 0 only partly");
        map.mark_present(range(2 * BLOCK_BYTES, 10));
        assert_eq!(map.present_bytes(), 10, "last block up to the end");
        map.mark_present(range(0, size));
        assert!(map.is_complete());
        assert_eq!(map.present_bytes(), size);
    }

    #[test]
    fn bitmap_round_trips_and_rejects_wrong_length() {
        let mut map = BlockMap::empty(20 * BLOCK_BYTES);
        map.mark_present(range(9 * BLOCK_BYTES, 2 * BLOCK_BYTES));
        let restored = BlockMap::from_bytes(map.size(), map.as_bytes().to_vec());
        assert_eq!(restored, map);
        let broken = BlockMap::from_bytes(map.size(), vec![0xff]);
        assert_eq!(broken, BlockMap::empty(map.size()));
        // Surplus bits after the last block are ignored.
        let padded = BlockMap::from_bytes(3 * BLOCK_BYTES, vec![0xff]);
        assert_eq!(padded, BlockMap::complete(3 * BLOCK_BYTES));
    }

    #[test]
    fn growing_keeps_old_gaps_and_marks_new_data() {
        let mut map = BlockMap::empty(BLOCK_BYTES + 10);
        map.resize(3 * BLOCK_BYTES);
        assert_eq!(
            map.missing(range(0, 3 * BLOCK_BYTES)),
            vec![range(0, 2 * BLOCK_BYTES)],
            "partial old block is still missing"
        );
        let mut complete = BlockMap::complete(10);
        complete.resize(2 * BLOCK_BYTES);
        assert!(complete.is_complete());
    }

    #[test]
    fn shrinking_keeps_what_was_there() {
        let mut map = BlockMap::complete(3 * BLOCK_BYTES);
        map.resize(BLOCK_BYTES + 1);
        assert!(map.is_complete());
        map.resize(0);
        assert!(map.is_complete());
        assert_eq!(map.as_bytes(), &[] as &[u8]);
    }

    proptest! {
        /// After fetching all reported gaps nothing is missing in the wanted range any more,
        /// and the gaps lie within the file and do not overlap.
        #[test]
        fn fetching_the_missing_ranges_fills_the_request(
            size in 0u64..(10 * BLOCK_BYTES),
            present in proptest::collection::vec(any::<bool>(), 10),
            offset in 0u64..(11 * BLOCK_BYTES),
            length in 0u64..(4 * BLOCK_BYTES),
        ) {
            let mut map = BlockMap::empty(size);
            for (block, is_present) in (0u64..).zip(present) {
                if is_present {
                    map.mark_present(range(block * BLOCK_BYTES, BLOCK_BYTES));
                }
            }
            let wanted = range(offset, length);
            let gaps = map.missing(wanted);
            let mut previous_end = 0;
            for gap in &gaps {
                prop_assert!(gap.offset >= previous_end);
                prop_assert!(gap.end() <= size);
                prop_assert_eq!(gap.offset % BLOCK_BYTES, 0);
                previous_end = gap.end();
                map.mark_present(*gap);
            }
            prop_assert!(map.missing(wanted).is_empty());
        }
    }
}
