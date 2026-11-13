//! A radio's band filters and which one a frequency gets.

/// A band filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandFilter {
    pub label: &'static str,
    /// Passband, in kHz.
    pub min_khz: u32,
    pub max_khz: u32,
    /// Used when no filter covers the frequency.
    pub wide: bool,
}

impl BandFilter {
    pub fn covers(&self, hz: u64) -> bool {
        let khz = hz / 1000;
        u64::from(self.min_khz) <= khz && khz <= u64::from(self.max_khz)
    }

    fn width_khz(&self) -> u32 {
        self.max_khz.saturating_sub(self.min_khz)
    }
}

/// A frequency range, in kHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreqRange {
    pub min_khz: u32,
    pub max_khz: u32,
}

impl FreqRange {
    pub fn contains(&self, hz: u64) -> bool {
        u64::from(self.min_khz) * 1000 <= hz && hz <= u64::from(self.max_khz) * 1000
    }
}

/// A radio's filters and which one is in use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FilterState {
    /// Empty when the radio has no filter control.
    pub filters: &'static [BandFilter],
    /// An index into `filters`; `None` is auto.
    pub selected: Option<u8>,
    /// The filter in use.
    pub active: Option<u8>,
}

/// What a radio is built for. Informative only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BandRanges {
    pub sub_ghz: Option<FreqRange>,
    pub band_24: Option<FreqRange>,
}

/// Whether the table has filter `index`.
pub fn exists(table: &[BandFilter], index: u8) -> bool {
    usize::from(index) < table.len()
}

/// The wide filter.
pub fn widest(table: &[BandFilter]) -> Option<u8> {
    let index = table.iter().position(|filter| filter.wide).unwrap_or(0);
    (index < table.len()).then_some(index as u8)
}

/// The narrowest filter that covers `hz`, else the wide one.
pub fn auto(table: &[BandFilter], hz: u64) -> Option<u8> {
    table
        .iter()
        .enumerate()
        .filter(|(_, filter)| filter.covers(hz))
        .min_by_key(|(_, filter)| filter.width_khz())
        .map(|(index, _)| index as u8)
        .or_else(|| widest(table))
}

/// The selected filter, or the auto choice. `wide` forces the wide filter in auto.
pub fn resolve(table: &[BandFilter], selected: Option<u8>, hz: u64, wide: bool) -> Option<u8> {
    match selected {
        Some(index) if exists(table, index) => Some(index),
        _ if wide => widest(table),
        _ => auto(table, hz),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MHZ: u64 = 1_000_000;

    const TABLE: [BandFilter; 3] = [
        BandFilter { label: "Wide", min_khz: 700_000, max_khz: 1_000_000, wide: true },
        BandFilter { label: "868", min_khz: 862_000, max_khz: 876_000, wide: false },
        BandFilter { label: "915", min_khz: 902_000, max_khz: 928_000, wide: false },
    ];

    #[test]
    fn auto_takes_the_narrowest_that_covers_else_the_wide_one() {
        assert_eq!(auto(&TABLE, 868 * MHZ), Some(1));
        assert_eq!(auto(&TABLE, 915 * MHZ), Some(2));
        assert_eq!(auto(&TABLE, 800 * MHZ), Some(0));
        assert_eq!(auto(&TABLE, 433 * MHZ), Some(0));
        assert!(!TABLE[0].covers(433 * MHZ));
        assert_eq!(auto(&[], 868 * MHZ), None);
    }

    #[test]
    fn a_selection_wins_and_wide_only_overrides_auto() {
        assert_eq!(resolve(&TABLE, Some(2), 868 * MHZ, false), Some(2));
        assert_eq!(resolve(&TABLE, None, 868 * MHZ, true), Some(0));
        assert_eq!(resolve(&TABLE, None, 868 * MHZ, false), Some(1));
        // An index the board does not have is auto.
        assert_eq!(resolve(&TABLE, Some(7), 868 * MHZ, false), Some(1));
        assert!(!exists(&TABLE, 3));
    }

    #[test]
    fn a_range_contains_its_ends() {
        let range = FreqRange { min_khz: 863_000, max_khz: 928_000 };
        assert!(range.contains(863 * MHZ) && range.contains(928 * MHZ));
        assert!(!range.contains(929 * MHZ));
    }
}
