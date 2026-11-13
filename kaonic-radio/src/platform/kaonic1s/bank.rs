//! Each board's filters and the FLT_SEL_V1/V2 lines that select them.

use crate::filter::{BandFilter, FreqRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Low,
    High,
}

/// The FLT_SEL_V1/V2 levels for one filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
    pub v1: Level,
    pub v2: Level,
}

use Level::{High, Low};

/// A board's filters and how to select them.
#[derive(Debug, Clone, Copy)]
pub struct FilterBank {
    pub filters: &'static [BandFilter],
    select: fn(u8) -> Option<Lines>,
}

impl FilterBank {
    pub fn lines(&self, index: u8) -> Option<Lines> {
        if usize::from(index) < self.filters.len() { (self.select)(index) } else { None }
    }

    /// The filter these lines select.
    pub fn by_lines(&self, lines: Lines) -> Option<u8> {
        (0..self.filters.len() as u8).find(|&index| self.lines(index) == Some(lines))
    }
}

/// The transceiver's own sub-GHz range, for the prototypes.
pub const CHIP_SUB_GHZ: FreqRange = FreqRange { min_khz: 389_500, max_khz: 1_020_000 };
pub const BAND_24: FreqRange = FreqRange { min_khz: 2_400_000, max_khz: 2_483_500 };
/// Kaonic 1S r24/r30.
pub const KAONIC1S_SUB_GHZ: FreqRange = FreqRange { min_khz: 863_000, max_khz: 928_000 };
/// Kaonic 1X r09.
pub const KAONIC1X_SUB_GHZ: FreqRange = FreqRange { min_khz: 390_000, max_khz: 510_000 };

// The hub stores filter indices: append, never reorder.
// V1=Low, V2=Low shuts the switch down, so no filter uses it.

const fn filter(label: &'static str, min_khz: u32, max_khz: u32, wide: bool) -> BandFilter {
    BandFilter { label, min_khz, max_khz, wide }
}

/// The prototypes (not confirmed against their schematics).
pub const PROTO: FilterBank = FilterBank {
    filters: &[filter("Wide", 389_500, 1_020_000, true), filter("868 MHz", 862_000, 876_000, false)],
    select: |index| match index {
        0 => Some(Lines { v1: High, v2: Low }),
        1 => Some(Lines { v1: Low, v2: High }),
        _ => None,
    },
};

/// Kaonic 1S r24 and r30, both radios.
fn kaonic1s_select(index: u8) -> Option<Lines> {
    match index {
        // Ceramic band-pass.
        0 => Some(Lines { v1: High, v2: Low }),
        // 868 MHz SAW.
        1 => Some(Lines { v1: Low, v2: High }),
        // 915 MHz SAW.
        2 => Some(Lines { v1: High, v2: High }),
        _ => None,
    }
}

pub const KAONIC1S_R24: FilterBank = FilterBank {
    filters: &[
        filter("Wide 729–960 MHz", 729_000, 960_000, true),
        filter("868 MHz", 862_000, 876_000, false),
        filter("915 MHz", 902_000, 928_000, false),
    ],
    select: kaonic1s_select,
};

pub const KAONIC1S_R30: FilterBank = FilterBank {
    filters: &[
        filter("Wide 730–960 MHz", 730_000, 960_000, true),
        filter("868 MHz", 862_000, 876_000, false),
        filter("915 MHz", 902_000, 928_000, false),
    ],
    select: kaonic1s_select,
};

/// Kaonic 1X r09, both radios.
pub const KAONIC1X_R09: FilterBank = FilterBank {
    filters: &[
        filter("Wide 330–565 MHz", 330_000, 565_000, true),
        filter("445–475 MHz", 445_000, 475_000, false),
        filter("424–442 MHz", 424_000, 442_000, false),
    ],
    select: |index| match index {
        // Discrete band-pass.
        0 => Some(Lines { v1: Low, v2: High }),
        // 445-475 MHz SAW.
        1 => Some(Lines { v1: High, v2: Low }),
        // 424-442 MHz SAW.
        2 => Some(Lines { v1: High, v2: High }),
        _ => None,
    },
};

/// `KAONIC_RF09_FLT_<n>=10` forces V1 high, V2 low (bring-up).
pub fn parse_lines(text: &str) -> Option<Lines> {
    let level = |byte: u8| match byte {
        b'0' => Some(Low),
        b'1' => Some(High),
        _ => None,
    };
    match text.as_bytes() {
        [v1, v2] => Some(Lines { v1: level(*v1)?, v2: level(*v2)? }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::auto;

    const MHZ: u64 = 1_000_000;
    const BANKS: [FilterBank; 4] = [PROTO, KAONIC1S_R24, KAONIC1S_R30, KAONIC1X_R09];

    #[test]
    fn every_filter_has_its_own_lines_and_none_shuts_the_switch() {
        for bank in BANKS {
            assert!(bank.filters[0].wide, "wide first");
            assert_eq!(bank.filters.iter().filter(|filter| filter.wide).count(), 1);
            let mut seen = Vec::new();
            for index in 0..bank.filters.len() as u8 {
                let lines = bank.lines(index).expect("every filter has lines");
                assert_ne!(lines, Lines { v1: Low, v2: Low }, "never shut the switch down");
                assert!(!seen.contains(&lines), "one filter per line setting");
                seen.push(lines);
            }
            assert_eq!(bank.lines(bank.filters.len() as u8), None);
        }
    }

    #[test]
    fn the_1s_keeps_the_lines_commd_always_used() {
        for bank in [KAONIC1S_R24, KAONIC1S_R30] {
            assert_eq!(bank.lines(0), Some(Lines { v1: High, v2: Low }), "wide");
            assert_eq!(bank.lines(1), Some(Lines { v1: Low, v2: High }), "868 MHz");
            assert_eq!(auto(bank.filters, 868_100_000), Some(1));
            assert_eq!(auto(bank.filters, 915 * MHZ), Some(2));
            assert_eq!(auto(bank.filters, 800 * MHZ), Some(0));
        }
        assert_eq!(KAONIC1S_R24.filters[0].min_khz, 729_000);
        assert_eq!(KAONIC1S_R30.filters[0].min_khz, 730_000);
        assert_eq!(auto(PROTO.filters, 869_535_000), Some(1));
        assert_eq!(auto(PROTO.filters, 915 * MHZ), Some(0));
    }

    #[test]
    fn r09_auto_takes_the_narrowest_saw_else_wide() {
        let bank = KAONIC1X_R09;
        assert_eq!(auto(bank.filters, 433_920_000), Some(2));
        assert_eq!(auto(bank.filters, 460 * MHZ), Some(1));
        assert_eq!(auto(bank.filters, 400 * MHZ), Some(0));
        assert_eq!(auto(bank.filters, 600 * MHZ), Some(0));
        assert_eq!(bank.lines(0), Some(Lines { v1: Low, v2: High }));
    }

    #[test]
    fn the_override_maps_lines_back_to_a_filter() {
        assert_eq!(parse_lines("10"), Some(Lines { v1: High, v2: Low }));
        assert_eq!(parse_lines("1"), None);
        assert_eq!(parse_lines("12"), None);
        assert_eq!(KAONIC1S_R30.by_lines(Lines { v1: High, v2: High }), Some(2));
        assert_eq!(KAONIC1S_R30.by_lines(Lines { v1: Low, v2: Low }), None);
        assert_eq!(PROTO.by_lines(Lines { v1: High, v2: High }), None);
    }

    #[test]
    fn board_ranges() {
        assert!(KAONIC1X_SUB_GHZ.contains(433_920_000));
        assert!(!KAONIC1X_SUB_GHZ.contains(868 * MHZ));
        assert!(KAONIC1S_SUB_GHZ.contains(863 * MHZ) && KAONIC1S_SUB_GHZ.contains(928 * MHZ));
        assert!(BAND_24.contains(2_450 * MHZ));
    }
}
