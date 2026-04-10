//! Finding a value you cannot see the address of.
//!
//! The loop everyone knows from Cheat Engine: search for 100, take some
//! damage, search again for what it is now, repeat until one address is left.
//! Each round throws away addresses that did not behave the way you said they
//! would.
//!
//! Between rounds this keeps a copy of the bytes it last saw rather than a
//! list of addresses. A first scan on a game with no idea what the number is
//! leaves tens of millions of candidates, and a list of those costs more
//! memory than a copy of the pages themselves.

use rayon::prelude::*;

use crate::error::Result;
use crate::region::Region;
use crate::target::Target;
use crate::value::{Scalar, ValueKind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filter {
    /// Everything, for when you do not know the starting number.
    Unknown,
    Exact(Scalar),
    Changed,
    Unchanged,
    Increased,
    Decreased,
    Between(Scalar, Scalar),
}

impl Filter {
    fn keep(&self, old: Scalar, new: Scalar) -> bool {
        match self {
            Filter::Unknown => true,
            Filter::Exact(want) => new.matches(*want),
            Filter::Changed => !new.matches(old),
            Filter::Unchanged => new.matches(old),
            Filter::Increased => new.as_f64() > old.as_f64(),
            Filter::Decreased => new.as_f64() < old.as_f64(),
            Filter::Between(low, high) => {
                new.as_f64() >= low.as_f64() && new.as_f64() <= high.as_f64()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub addr: usize,
    pub value: Scalar,
}

struct Snapshot {
    base: usize,
    /// What the page held at the end of the previous round.
    previous: Vec<u8>,
    /// One flag per aligned slot. False means this address is out.
    live: Vec<bool>,
}

impl Snapshot {
    fn count(&self) -> usize {
        self.live.iter().filter(|l| **l).count()
    }
}

pub struct Search {
    pub kind: ValueKind,
    align: usize,
    snapshots: Vec<Snapshot>,
    rounds: usize,
}

impl Search {
    /// Values are aligned to their own size in practice, which cuts the number
    /// of slots to check by four for a 32-bit value. A game storing an int on
    /// an odd address would be missed, and that is rare enough to be worth it.
    fn slots(region_len: usize, align: usize, size: usize) -> usize {
        if region_len < size {
            0
        } else {
            (region_len - size) / align + 1
        }
    }

    pub fn first(target: &dyn Target, kind: ValueKind, filter: Filter) -> Result<Self> {
        let align = kind.size();
        let regions: Vec<Region> = target
            .regions()?
            .into_iter()
            .filter(Region::scannable_data)
            .collect();

        let snapshots: Vec<Snapshot> = regions
            .par_iter()
            .filter_map(|region| {
                let mut bytes = vec![0u8; region.size];
                target.read_into(region.base, &mut bytes).ok()?;

                let slots = Self::slots(bytes.len(), align, kind.size());
                let mut live = vec![false; slots];
                for (slot, flag) in live.iter_mut().enumerate() {
                    let at = slot * align;
                    if let Some(value) = kind.read(&bytes[at..]) {
                        // Nothing to compare against yet, so old and new are
                        // the same reading.
                        *flag = filter.keep(value, value);
                    }
                }

                Some(Snapshot {
                    base: region.base,
                    previous: bytes,
                    live,
                })
            })
            .collect();

        Ok(Self {
            kind,
            align,
            snapshots,
            rounds: 1,
        })
    }

    /// Narrow the candidates using how each one changed since the last round.
    pub fn next(&mut self, target: &dyn Target, filter: Filter) -> Result<()> {
        let kind = self.kind;
        let align = self.align;

        self.snapshots.par_iter_mut().for_each(|snapshot| {
            let mut current = vec![0u8; snapshot.previous.len()];
            if target.read_into(snapshot.base, &mut current).is_err() {
                // Page went away, so every candidate in it is gone too.
                snapshot.live.iter_mut().for_each(|l| *l = false);
                return;
            }

            for (slot, flag) in snapshot.live.iter_mut().enumerate() {
                if !*flag {
                    continue;
                }
                let at = slot * align;
                match (
                    kind.read(&snapshot.previous[at..]),
                    kind.read(&current[at..]),
                ) {
                    (Some(old), Some(new)) => *flag = filter.keep(old, new),
                    _ => *flag = false,
                }
            }

            snapshot.previous = current;
        });

        self.rounds += 1;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.snapshots.par_iter().map(Snapshot::count).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn rounds(&self) -> usize {
        self.rounds
    }

    /// Surviving addresses with their last known value, capped so a UI asking
    /// for results after one round does not try to render ten million rows.
    pub fn results(&self, limit: usize) -> Vec<Candidate> {
        let mut out = Vec::new();
        for snapshot in &self.snapshots {
            for (slot, live) in snapshot.live.iter().enumerate() {
                if !*live {
                    continue;
                }
                let at = slot * self.align;
                if let Some(value) = self.kind.read(&snapshot.previous[at..]) {
                    out.push(Candidate {
                        addr: snapshot.base + at,
                        value,
                    });
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockTarget;

    const BASE: usize = 0x2000_0000;

    fn target() -> MockTarget {
        MockTarget::zeroed(BASE, 4096)
    }

    fn put(target: &MockTarget, offset: usize, value: i32) {
        target.poke(BASE + offset, &value.to_ne_bytes());
    }

    #[test]
    fn exact_scan_finds_the_address() {
        let t = target();
        put(&t, 64, 1337);

        let search = Search::first(&t, ValueKind::I32, Filter::Exact(Scalar::I32(1337))).unwrap();

        let hits = search.results(16);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].addr, BASE + 64);
        assert_eq!(hits[0].value, Scalar::I32(1337));
    }

    #[test]
    fn unknown_scan_keeps_every_aligned_slot() {
        let t = MockTarget::zeroed(BASE, 400);
        let search = Search::first(&t, ValueKind::I32, Filter::Unknown).unwrap();
        assert_eq!(search.len(), (400 - 4) / 4 + 1);
    }

    #[test]
    fn narrowing_by_new_value_leaves_one_address() {
        let t = target();
        put(&t, 64, 100);
        put(&t, 128, 100);
        put(&t, 256, 100);

        let mut search =
            Search::first(&t, ValueKind::I32, Filter::Exact(Scalar::I32(100))).unwrap();
        assert_eq!(search.len(), 3);

        // Only one of them drops, the way health would after taking a hit.
        put(&t, 64, 80);
        search.next(&t, Filter::Exact(Scalar::I32(80))).unwrap();

        assert_eq!(search.len(), 1);
        assert_eq!(search.results(4)[0].addr, BASE + 64);
    }

    #[test]
    fn decreased_filter_works_without_knowing_the_number() {
        let t = target();
        put(&t, 32, 500);
        put(&t, 96, 500);

        let mut search = Search::first(&t, ValueKind::I32, Filter::Unknown).unwrap();
        put(&t, 32, 480);
        search.next(&t, Filter::Decreased).unwrap();

        let hits = search.results(64);
        assert!(hits.iter().any(|c| c.addr == BASE + 32));
        assert!(!hits.iter().any(|c| c.addr == BASE + 96));
    }

    #[test]
    fn unchanged_filter_drops_the_one_that_moved() {
        let t = target();
        put(&t, 16, 7);
        put(&t, 48, 7);

        let mut search = Search::first(&t, ValueKind::I32, Filter::Exact(Scalar::I32(7))).unwrap();
        put(&t, 16, 9);
        search.next(&t, Filter::Unchanged).unwrap();

        let hits = search.results(8);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].addr, BASE + 48);
    }

    #[test]
    fn increased_filter_survives_several_rounds() {
        let t = target();
        put(&t, 20, 10);

        let mut search = Search::first(&t, ValueKind::I32, Filter::Unknown).unwrap();
        for step in [20, 30, 45] {
            put(&t, 20, step);
            search.next(&t, Filter::Increased).unwrap();
        }

        assert!(search.results(64).iter().any(|c| c.addr == BASE + 20));
        assert_eq!(search.rounds(), 4);
    }

    #[test]
    fn between_filter_bounds_the_result() {
        let t = target();
        put(&t, 40, 55);
        put(&t, 80, 5000);

        let search = Search::first(
            &t,
            ValueKind::I32,
            Filter::Between(Scalar::I32(50), Scalar::I32(60)),
        )
        .unwrap();

        let hits = search.results(64);
        assert!(hits.iter().any(|c| c.addr == BASE + 40));
        assert!(!hits.iter().any(|c| c.addr == BASE + 80));
    }

    #[test]
    fn results_are_capped() {
        let t = MockTarget::zeroed(BASE, 4096);
        let search = Search::first(&t, ValueKind::I32, Filter::Unknown).unwrap();
        assert_eq!(search.results(10).len(), 10);
    }

    #[test]
    fn a_search_can_end_up_empty() {
        let t = target();
        put(&t, 64, 100);

        let mut search =
            Search::first(&t, ValueKind::I32, Filter::Exact(Scalar::I32(100))).unwrap();
        search.next(&t, Filter::Exact(Scalar::I32(999))).unwrap();

        assert!(search.is_empty());
        assert!(search.results(8).is_empty());
    }

    #[test]
    fn floats_are_found_despite_imprecision() {
        let t = target();
        t.poke(BASE + 100, &99.99999f32.to_ne_bytes());

        let search = Search::first(&t, ValueKind::F32, Filter::Exact(Scalar::F32(100.0))).unwrap();
        assert!(search.results(8).iter().any(|c| c.addr == BASE + 100));
    }
}
