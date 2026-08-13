//! Byte patterns with wildcards.
//!
//! A pattern is how a cheat survives a game update. Hardcoded addresses move
//! every time the binary is rebuilt, but the instruction bytes around the
//! interesting code usually do not, so we search for those and blank out the
//! parts that do change, such as offsets and relative jumps.

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    bytes: Vec<u8>,
    /// Parallel to `bytes`. False means "anything goes here".
    fixed: Vec<bool>,
    /// Index of the first non-wildcard byte, used to skip ahead cheaply.
    anchor: usize,
}

impl Pattern {
    pub fn parse(text: &str) -> Result<Self> {
        let mut bytes = Vec::new();
        let mut fixed = Vec::new();

        // cheat engine takes either character for a wildcard and tables use
        // both, often in the same file
        let any = |c: char| c == '?' || c == '*';

        for token in text.split_whitespace() {
            if token.chars().all(any) {
                // "?" and "??" both mean one wildcard byte.
                bytes.push(0);
                fixed.push(false);
                continue;
            }

            if token.len() % 2 != 0 || !token.chars().all(|c| c.is_ascii_hexdigit() || any(c)) {
                return Err(Error::BadPattern(format!("bad token {token:?}")));
            }

            // Allow "488B0D" as well as "48 8B 0D".
            for pair in token.as_bytes().chunks(2) {
                let pair = std::str::from_utf8(pair).expect("ascii checked above");
                if pair.contains(any) {
                    bytes.push(0);
                    fixed.push(false);
                } else {
                    let byte = u8::from_str_radix(pair, 16)
                        .map_err(|_| Error::BadPattern(format!("bad byte {pair:?}")))?;
                    bytes.push(byte);
                    fixed.push(true);
                }
            }
        }

        if bytes.is_empty() {
            return Err(Error::BadPattern("pattern is empty".into()));
        }
        let anchor = fixed
            .iter()
            .position(|f| *f)
            .ok_or_else(|| Error::BadPattern("pattern is all wildcards".into()))?;

        Ok(Self {
            bytes,
            fixed,
            anchor,
        })
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn matches_at(&self, haystack: &[u8], at: usize) -> bool {
        let Some(window) = haystack.get(at..at + self.bytes.len()) else {
            return false;
        };
        window
            .iter()
            .zip(&self.bytes)
            .zip(&self.fixed)
            .all(|((got, want), fixed)| !fixed || got == want)
    }

    /// Every offset in `haystack` where this pattern matches.
    pub fn find_all(&self, haystack: &[u8]) -> Vec<usize> {
        let mut hits = Vec::new();
        if haystack.len() < self.bytes.len() {
            return hits;
        }

        let needle = self.bytes[self.anchor];
        let last_start = haystack.len() - self.bytes.len();

        // Jump between occurrences of the first fixed byte instead of testing
        // every offset. On a multi-megabyte region that is the difference
        // between a scan you notice and one you do not.
        let mut cursor = self.anchor;
        while cursor < haystack.len() {
            let Some(found) = memchr::memchr(needle, &haystack[cursor..]) else {
                break;
            };
            let hit = cursor + found;
            if hit < self.anchor {
                cursor = hit + 1;
                continue;
            }
            let start = hit - self.anchor;
            if start > last_start {
                break;
            }
            if self.matches_at(haystack, start) {
                hits.push(start);
            }
            cursor = hit + 1;
        }
        hits
    }

    pub fn find_first(&self, haystack: &[u8]) -> Option<usize> {
        self.find_all(haystack).into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spaced_hex() {
        let p = Pattern::parse("48 8B 05").unwrap();
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn parses_unspaced_hex() {
        assert_eq!(
            Pattern::parse("488B05").unwrap(),
            Pattern::parse("48 8B 05").unwrap()
        );
    }

    #[test]
    fn treats_single_and_double_question_marks_alike() {
        assert_eq!(
            Pattern::parse("48 ? 05").unwrap(),
            Pattern::parse("48 ?? 05").unwrap()
        );
    }

    #[test]
    fn rejects_rubbish() {
        assert!(Pattern::parse("48 ZZ").is_err());
        assert!(Pattern::parse("").is_err());
        assert!(Pattern::parse("?? ??").is_err());
        assert!(Pattern::parse("4").is_err());
    }

    #[test]
    fn finds_an_exact_match() {
        let hay = [0x00, 0x48, 0x8B, 0x05, 0xFF];
        assert_eq!(Pattern::parse("48 8B 05").unwrap().find_all(&hay), vec![1]);
    }

    #[test]
    fn wildcards_match_anything() {
        let hay = [0x48, 0x11, 0x05, 0x48, 0x99, 0x05];
        assert_eq!(
            Pattern::parse("48 ?? 05").unwrap().find_all(&hay),
            vec![0, 3]
        );
    }

    /* cheat engine takes a star as well, and plenty of tables use it. every
    one of those used to come back as a signature that is not in the game */
    #[test]
    fn a_star_is_a_wildcard_too() {
        let hay = [0x48, 0x11, 0x05, 0x48, 0x99, 0x05];
        for text in ["48 ** 05", "48 * 05", "48 *? 05"] {
            assert_eq!(
                Pattern::parse(text).unwrap().find_all(&hay),
                vec![0, 3],
                "{text}"
            );
        }
    }

    #[test]
    fn a_star_run_still_has_to_be_bytes_wide() {
        assert!(Pattern::parse("48 *0 05").is_ok());
        assert!(Pattern::parse("48 zz 05").is_err());
    }

    #[test]
    fn finds_a_match_at_the_very_start_and_end() {
        let hay = [0xAA, 0xBB, 0xCC, 0xAA, 0xBB];
        assert_eq!(Pattern::parse("AA BB").unwrap().find_all(&hay), vec![0, 3]);
    }

    #[test]
    fn overlapping_matches_are_all_reported() {
        let hay = [0xAA, 0xAA, 0xAA];
        assert_eq!(Pattern::parse("AA AA").unwrap().find_all(&hay), vec![0, 1]);
    }

    #[test]
    fn no_match_when_the_tail_is_short() {
        let hay = [0x48, 0x8B];
        assert!(Pattern::parse("48 8B 05")
            .unwrap()
            .find_all(&hay)
            .is_empty());
    }

    #[test]
    fn leading_wildcard_does_not_shift_the_result() {
        let hay = [0x00, 0x11, 0x48, 0x8B];
        // Anchor is the 0x48, but the match starts one byte earlier.
        assert_eq!(Pattern::parse("?? 48 8B").unwrap().find_all(&hay), vec![1]);
    }

    #[test]
    fn leading_wildcard_cannot_underflow_at_offset_zero() {
        let hay = [0x48, 0x8B, 0x48, 0x8B];
        assert_eq!(Pattern::parse("?? 48 8B").unwrap().find_all(&hay), vec![1]);
    }
}
