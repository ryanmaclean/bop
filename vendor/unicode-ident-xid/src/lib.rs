// SPDX-License-Identifier: MIT OR Apache-2.0
//! Compatibility API backed by unicode-xid's expressly licensed Unicode 17 tables.
//! This is a separate implementation, not the upstream unicode-ident crate.
#![no_std]
#![forbid(unsafe_code)]

#[rustfmt::skip]
mod tables;

/// Unicode version used by the identifier properties.
pub const UNICODE_VERSION: (u8, u8, u8) = (
    tables::UNICODE_VERSION.0 as u8,
    tables::UNICODE_VERSION.1 as u8,
    tables::UNICODE_VERSION.2 as u8,
);

/// Whether a scalar has the Unicode XID_Start property.
#[inline]
pub fn is_xid_start(ch: char) -> bool {
    tables::derived_property::XID_Start(ch)
}

/// Whether a scalar has the Unicode XID_Continue property.
#[inline]
pub fn is_xid_continue(ch: char) -> bool {
    tables::derived_property::XID_Continue(ch)
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{format, vec::Vec};

    // An independent forward range sweep exercises the binary-search lookup
    // across every scalar, including range boundaries and noncharacters.
    // It validates lookup behavior, not the upstream Unicode data's provenance.
    fn ranges(name: &str) -> Vec<(u32, u32)> {
        let source = include_str!("tables.rs");
        let marker = format!("static {name}_table:");
        let section = source
            .split_once(&marker)
            .unwrap()
            .1
            .split_once("];")
            .unwrap()
            .0;
        let points: Vec<u32> = section
            .split("\\u{")
            .skip(1)
            .map(|part| u32::from_str_radix(part.split_once('}').unwrap().0, 16).unwrap())
            .collect();
        assert_eq!(points.len() % 2, 0);
        let rows: Vec<_> = points.chunks_exact(2).map(|p| (p[0], p[1])).collect();
        assert!(rows.len() > 600);
        for &(lo, hi) in &rows {
            assert!(lo <= hi && hi <= 0x10ffff);
        }
        for pair in rows.windows(2) {
            assert!(pair[0].1 < pair[1].0);
        }
        rows
    }

    #[test]
    fn exhaustive_scalar_lookup_matches_forward_sweep() {
        for (name, lookup) in [
            ("XID_Start", is_xid_start as fn(char) -> bool),
            ("XID_Continue", is_xid_continue as fn(char) -> bool),
        ] {
            let rows = ranges(name);
            let mut index = 0;
            let mut observed = 0;
            for point in 0..=0x10ffff {
                let Some(ch) = char::from_u32(point) else {
                    continue;
                };
                while index < rows.len() && rows[index].1 < point {
                    index += 1;
                }
                let expected = index < rows.len() && rows[index].0 <= point;
                assert_eq!(lookup(ch), expected, "{name} U+{point:04X}");
                observed += 1;
            }
            assert_eq!(observed, 1_112_064);
        }
    }

    #[test]
    fn version_and_identifier_boundaries() {
        assert_eq!(UNICODE_VERSION, (17, 0, 0));
        assert_eq!(tables::UNICODE_VERSION, (17, 0, 0));
        for ch in ['A', 'z', 'é', '中', '\u{1000d}', '\u{1e6c0}'] {
            assert!(is_xid_start(ch));
            assert!(is_xid_continue(ch));
        }
        for ch in ['_', '0', '\u{301}'] {
            assert!(!is_xid_start(ch));
            assert!(is_xid_continue(ch));
        }
        for ch in ['\0', ' ', '-', '🂡', '\u{ffff}', '\u{10ffff}'] {
            assert!(!is_xid_start(ch));
            assert!(!is_xid_continue(ch));
        }
    }

    #[test]
    fn every_start_is_a_continue() {
        for ch in (0..=0x10ffff).filter_map(char::from_u32) {
            assert!(!is_xid_start(ch) || is_xid_continue(ch));
        }
    }
}
