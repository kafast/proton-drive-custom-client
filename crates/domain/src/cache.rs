//! Block-Zwischenspeicher für Dateien im virtuellen Laufwerk.
//!
//! Eine Datei wird in Blöcke fester Größe geteilt. [`BlockMap`] merkt sich,
//! welche Blöcke lokal vorliegen. So muss beim Lesen nur geholt werden, was
//! fehlt, und große Dateien sind sofort benutzbar.

/// Größe eines Blocks: 4 MiB. Groß genug für wenige Anfragen bei Proton,
/// klein genug, damit kurzes Hineinlesen (z. B. Vorschaubilder) schnell ist.
pub const BLOCK_BYTES: u64 = 4 * 1024 * 1024;

/// Ein zusammenhängender Bereich einer Datei in Bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// Start in Bytes ab Dateianfang.
    pub offset: u64,
    /// Länge in Bytes.
    pub length: u64,
}

impl ByteRange {
    /// Erstes Byte hinter dem Bereich.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.offset.saturating_add(self.length)
    }
}

/// Welche Blöcke einer Datei lokal vorliegen.
///
/// Ein Block gilt als vorhanden, wenn sein ganzer Inhalt (am Dateiende: bis
/// zum Ende) lokal steht, sei es geladen oder lokal geschrieben.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockMap {
    size: u64,
    /// Ein Bit je Block, niederwertiges Bit zuerst.
    bits: Vec<u8>,
}

/// Anzahl Blöcke für `size` Bytes.
fn block_count(size: u64) -> u64 {
    size.div_ceil(BLOCK_BYTES)
}

/// Anzahl Bytes für `blocks` Bits.
fn bitmap_len(blocks: u64) -> usize {
    // Bei realistischen Dateigrößen (unter 2^64 Bytes / 4 MiB / 8) passt das
    // immer in `usize`; sonst bleibt die Bitmap leer und alles gilt als fehlend.
    usize::try_from(blocks.div_ceil(8)).unwrap_or(0)
}

impl BlockMap {
    /// Nichts liegt vor (frisch angelegter Platzhalter).
    #[must_use]
    pub fn empty(size: u64) -> Self {
        Self {
            size,
            bits: vec![0; bitmap_len(block_count(size))],
        }
    }

    /// Alles liegt vor (z. B. eine lokal neu angelegte Datei).
    #[must_use]
    pub fn complete(size: u64) -> Self {
        let mut map = Self::empty(size);
        map.mark_present(ByteRange {
            offset: 0,
            length: size,
        });
        map
    }

    /// Stellt eine gespeicherte Bitmap wieder her. Passt die Länge nicht zur
    /// Größe (beschädigter Eintrag), gilt vorsichtshalber alles als fehlend.
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

    /// Die Bitmap zum Speichern.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    /// Dateigröße in Bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// `true`, wenn alle Blöcke vorliegen.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        (0..block_count(self.size)).all(|block| self.has(block))
    }

    /// Wie viele Bytes lokal vorliegen (belegter Platz im Zwischenspeicher).
    #[must_use]
    pub fn present_bytes(&self) -> u64 {
        (0..block_count(self.size))
            .filter(|&block| self.has(block))
            .map(|block| self.block_range(block).length)
            .sum()
    }

    /// Die fehlenden Teile von `wanted`, auf ganze Blöcke erweitert und
    /// zusammenhängende Blöcke zusammengefasst; am Dateiende abgeschnitten.
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

    /// Vermerkt alle Blöcke, die `range` vollständig abdeckt, als vorhanden.
    /// Der letzte Block zählt als abgedeckt, wenn `range` bis zum Dateiende reicht.
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

    /// Ändert die Dateigröße (Abschneiden oder Verlängern durch Schreiben).
    ///
    /// Beim Verlängern liegt der neue Teil lokal vor (er ist lokal entstanden).
    /// Der bisher letzte Block bleibt nur vorhanden, wenn er es schon war.
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
                // Der angebrochene alte Block fehlt noch teilweise.
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
        // `block % 8` ist immer kleiner als 8; das Bit passt also in ein Byte.
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

    /// Bits hinter dem letzten Block löschen, damit gleiche Inhalte gleich vergleichen.
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
        assert_eq!(map.missing(range(size, 10)), vec![], "hinter dem Ende");
        assert_eq!(map.missing(range(0, 0)), vec![]);
    }

    #[test]
    fn only_fully_covered_blocks_count_as_present() {
        let size = 2 * BLOCK_BYTES + 10;
        let mut map = BlockMap::empty(size);
        map.mark_present(range(1, BLOCK_BYTES));
        assert_eq!(map.present_bytes(), 0, "Block 0 nur teilweise");
        map.mark_present(range(2 * BLOCK_BYTES, 10));
        assert_eq!(map.present_bytes(), 10, "letzter Block bis zum Ende");
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
        // Überzählige Bits hinter dem letzten Block werden ignoriert.
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
            "angebrochener alter Block fehlt weiter"
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
        /// Nach dem Holen aller gemeldeten Lücken fehlt im gewünschten Bereich nichts mehr,
        /// und die Lücken liegen innerhalb der Datei und überlappen sich nicht.
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
