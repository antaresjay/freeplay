//! Sweeping a target's memory for a byte pattern.

use rayon::prelude::*;

use crate::error::{Error, Result};
use crate::pattern::Pattern;
use crate::region::Region;
use crate::target::Target;

/// Read granularity. Big enough that the per-read overhead disappears, small
/// enough that scanning a 2GB region does not need 2GB of our own memory.
const CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub enum Scope {
    /// Executable pages. Where instructions live, so where signatures live.
    #[default]
    Code,
    /// Private writable pages. Where gameplay values live.
    Data,
    /// Inside one loaded module.
    Module(String),
    Everything,
}

impl Scope {
    fn accepts(&self, region: &Region) -> bool {
        match self {
            Scope::Code => region.scannable_code(),
            Scope::Data => region.scannable_data(),
            Scope::Module(_) | Scope::Everything => region.protection.scannable(),
        }
    }
}

fn regions_for(target: &dyn Target, scope: &Scope) -> Result<Vec<Region>> {
    let mut regions: Vec<Region> = target.regions()?.into_iter().filter(|r| scope.accepts(r)).collect();

    if let Scope::Module(name) = scope {
        let module = target.module(name)?;
        regions.retain(|r| r.base >= module.base && r.end() <= module.end());
    }
    Ok(regions)
}

fn scan_region(target: &dyn Target, region: &Region, pattern: &Pattern) -> Vec<usize> {
    let overlap = pattern.len().saturating_sub(1);
    let mut hits = Vec::new();
    let mut offset = 0usize;
    let mut buf = Vec::new();

    while offset < region.size {
        let len = CHUNK.min(region.size - offset);
        buf.resize(len, 0);

        // Pages can be freed between enumerating and reading them, and a game
        // that just unloaded a level does this constantly. Skip and continue
        // rather than failing the whole scan.
        if target.read_into(region.base + offset, &mut buf).is_ok() {
            hits.extend(pattern.find_all(&buf).into_iter().map(|at| region.base + offset + at));
        }

        if len < CHUNK {
            break;
        }
        offset += CHUNK - overlap;
    }

    hits
}

/// Every address matching `pattern`, sorted.
pub fn find_all(target: &dyn Target, pattern: &Pattern, scope: Scope) -> Result<Vec<usize>> {
    let regions = regions_for(target, &scope)?;

    let mut hits: Vec<usize> = regions
        .par_iter()
        .flat_map_iter(|region| scan_region(target, region, pattern))
        .collect();

    // Chunks overlap so a match straddling a boundary is found twice.
    hits.sort_unstable();
    hits.dedup();
    Ok(hits)
}

/// The single address matching `pattern`.
///
/// A signature that matches more than once is a bug in the signature, not a
/// choice for the caller to make, so this refuses rather than guessing.
pub fn find_one(target: &dyn Target, pattern: &Pattern, scope: Scope) -> Result<usize> {
    let hits = find_all(target, pattern, scope)?;
    match hits.len() {
        0 => Err(Error::NotFound),
        1 => Ok(hits[0]),
        found => Err(Error::Ambiguous { found }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockTarget;

    const BASE: usize = 0x1000_0000;

    fn target_with(bytes: &[u8], at: usize) -> MockTarget {
        let mut memory = vec![0xCCu8; 64 * 1024];
        memory[at..at + bytes.len()].copy_from_slice(bytes);
        MockTarget::new(BASE, memory).executable()
    }

    #[test]
    fn finds_a_pattern_in_a_region() {
        let target = target_with(&[0x48, 0x8B, 0x05, 0xDE, 0xAD], 4096);
        let pattern = Pattern::parse("48 8B 05").unwrap();

        let hits = find_all(&target, &pattern, Scope::Code).unwrap();
        assert_eq!(hits, vec![BASE + 4096]);
    }

    #[test]
    fn find_one_returns_the_address() {
        let target = target_with(&[0x11, 0x22, 0x33, 0x44], 200);
        let pattern = Pattern::parse("11 22 33 44").unwrap();
        assert_eq!(find_one(&target, &pattern, Scope::Code).unwrap(), BASE + 200);
    }

    #[test]
    fn find_one_refuses_when_nothing_matches() {
        let target = target_with(&[0x11], 0);
        let pattern = Pattern::parse("DE AD BE EF").unwrap();
        assert!(matches!(find_one(&target, &pattern, Scope::Code), Err(Error::NotFound)));
    }

    #[test]
    fn find_one_refuses_an_ambiguous_signature() {
        let mut memory = vec![0x00u8; 8192];
        memory[100..104].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        memory[900..904].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let target = MockTarget::new(BASE, memory).executable();
        let pattern = Pattern::parse("DE AD BE EF").unwrap();

        assert!(matches!(
            find_one(&target, &pattern, Scope::Code),
            Err(Error::Ambiguous { found: 2 })
        ));
    }

    #[test]
    fn code_scope_skips_plain_data_pages() {
        // Not executable, so a code scan should not look here.
        let target = target_with(&[0x48, 0x8B, 0x05], 512);
        let data_only = MockTarget::new(BASE, target.snapshot());
        let pattern = Pattern::parse("48 8B 05").unwrap();

        assert!(find_all(&data_only, &pattern, Scope::Code).unwrap().is_empty());
        assert_eq!(find_all(&data_only, &pattern, Scope::Data).unwrap(), vec![BASE + 512]);
    }

    #[test]
    fn a_match_spanning_a_chunk_boundary_is_found_once() {
        // Straddle the 4MB read boundary, which is where naive chunking either
        // misses the match or reports it twice.
        let mut memory = vec![0x00u8; CHUNK + 4096];
        let at = CHUNK - 2;
        memory[at..at + 4].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let target = MockTarget::new(BASE, memory).executable();

        let pattern = Pattern::parse("AA BB CC DD").unwrap();
        assert_eq!(find_all(&target, &pattern, Scope::Code).unwrap(), vec![BASE + at]);
    }

    #[test]
    fn wildcards_work_through_the_scanner() {
        let target = target_with(&[0x48, 0x8B, 0x0D, 0x11, 0x22, 0x33, 0x44], 300);
        let pattern = Pattern::parse("48 8B 0D ?? ?? ?? ??").unwrap();
        assert_eq!(find_one(&target, &pattern, Scope::Code).unwrap(), BASE + 300);
    }
}
