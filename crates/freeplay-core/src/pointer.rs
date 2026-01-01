//! Pointer chains.
//!
//! The address a value lives at moves every time you load a save, because the
//! object holding it was allocated somewhere new. What does not move is the
//! route to it: a fixed spot in the game's own executable holds a pointer,
//! which leads to a structure, which holds another pointer, and so on until
//! you reach the number. Writing that route down is how a cheat keeps working
//! across sessions.

use std::fmt;

use crate::error::{Error, Result};
use crate::target::Target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// Offset inside a loaded module, the usual case. Modules move under ASLR
    /// but the offset within them does not.
    Module { module: String, offset: usize },
    /// A literal address, only good for the session it was found in.
    Absolute(usize),
}

impl Anchor {
    fn address(&self, target: &dyn Target) -> Result<usize> {
        match self {
            Anchor::Module { module, offset } => Ok(target.module(module)?.base + offset),
            Anchor::Absolute(addr) => Ok(*addr),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerPath {
    pub anchor: Anchor,
    /// Applied after each dereference. The last one is added without a further
    /// read, since it lands on the value itself.
    pub hops: Vec<isize>,
}

impl PointerPath {
    pub fn module(module: impl Into<String>, offset: usize, hops: Vec<isize>) -> Self {
        Self { anchor: Anchor::Module { module: module.into(), offset }, hops }
    }

    pub fn absolute(addr: usize, hops: Vec<isize>) -> Self {
        Self { anchor: Anchor::Absolute(addr), hops }
    }

    /// Walk the chain and return the address the value sits at.
    ///
    /// A null or unreadable pointer part way through is normal rather than
    /// exceptional. It usually means you are at a menu and the object does not
    /// exist yet, which is why the error says which hop gave up.
    pub fn resolve(&self, target: &dyn Target) -> Result<usize> {
        let mut addr = self.anchor.address(target)?;

        addr = read_hop(target, addr, 0)?;

        for (index, offset) in self.hops.iter().enumerate() {
            addr = addr.wrapping_add_signed(*offset);
            let last = index + 1 == self.hops.len();
            if !last {
                addr = read_hop(target, addr, index + 1)?;
            }
        }

        if addr == 0 {
            return Err(Error::BrokenChain { hop: self.hops.len(), addr: 0 });
        }
        Ok(addr)
    }

    /// Whether the chain resolves right now, for greying out a toggle.
    pub fn is_live(&self, target: &dyn Target) -> bool {
        self.resolve(target).is_ok()
    }
}

fn read_hop(target: &dyn Target, addr: usize, hop: usize) -> Result<usize> {
    if addr == 0 {
        return Err(Error::BrokenChain { hop, addr });
    }
    let next = target.read_pointer(addr).map_err(|_| Error::BrokenChain { hop, addr })?;
    // User space on 64-bit Windows stops well below this. Anything larger is a
    // float or a string being misread as a pointer, so stop rather than
    // chasing it into nothing.
    if next == 0 || next > 0x7FFF_FFFF_FFFF {
        return Err(Error::BrokenChain { hop, addr });
    }
    Ok(next)
}

impl fmt::Display for PointerPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.anchor {
            Anchor::Module { module, offset } => write!(f, "{module}+{offset:#x}")?,
            Anchor::Absolute(addr) => write!(f, "{addr:#x}")?,
        }
        for hop in &self.hops {
            if *hop < 0 {
                write!(f, " -{:#x}", -hop)?;
            } else {
                write!(f, " +{hop:#x}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockTarget;
    use crate::value::{Scalar, ValueKind};

    const BASE: usize = 0x3000_0000;

    /// base+0x100 -> 0x3000_0500 -> +0x20 -> 0x3000_0900 -> +0x8 = value
    fn chained() -> MockTarget {
        let t = MockTarget::zeroed(BASE, 0x2000).with_module("game.exe", BASE, 0x2000);
        t.poke_usize(BASE + 0x100, BASE + 0x500);
        t.poke_usize(BASE + 0x520, BASE + 0x900);
        t.poke(BASE + 0x908, &4242i32.to_ne_bytes());
        t
    }

    #[test]
    fn resolves_a_two_hop_chain() {
        let t = chained();
        let path = PointerPath::module("game.exe", 0x100, vec![0x20, 0x8]);

        let addr = path.resolve(&t).unwrap();
        assert_eq!(addr, BASE + 0x908);
        assert_eq!(t.read_scalar(addr, ValueKind::I32).unwrap(), Scalar::I32(4242));
    }

    #[test]
    fn resolves_a_single_hop() {
        let t = MockTarget::zeroed(BASE, 0x1000).with_module("game.exe", BASE, 0x1000);
        t.poke_usize(BASE + 0x40, BASE + 0x200);
        t.poke(BASE + 0x210, &7i32.to_ne_bytes());

        let path = PointerPath::module("game.exe", 0x40, vec![0x10]);
        assert_eq!(path.resolve(&t).unwrap(), BASE + 0x210);
    }

    #[test]
    fn negative_offsets_work() {
        let t = MockTarget::zeroed(BASE, 0x1000).with_module("game.exe", BASE, 0x1000);
        t.poke_usize(BASE + 0x40, BASE + 0x300);
        let path = PointerPath::module("game.exe", 0x40, vec![-0x10]);
        assert_eq!(path.resolve(&t).unwrap(), BASE + 0x2F0);
    }

    #[test]
    fn a_null_pointer_reports_which_hop_broke() {
        let t = MockTarget::zeroed(BASE, 0x1000).with_module("game.exe", BASE, 0x1000);
        // Anchor holds null, which is what a menu looks like.
        let path = PointerPath::module("game.exe", 0x100, vec![0x20, 0x8]);

        match path.resolve(&t) {
            Err(Error::BrokenChain { hop, .. }) => assert_eq!(hop, 0),
            other => panic!("expected a broken chain, got {other:?}"),
        }
    }

    #[test]
    fn breaks_on_a_later_hop_too() {
        let t = MockTarget::zeroed(BASE, 0x2000).with_module("game.exe", BASE, 0x2000);
        t.poke_usize(BASE + 0x100, BASE + 0x500);
        // BASE+0x520 is still zero, so hop 1 is where it gives up.
        let path = PointerPath::module("game.exe", 0x100, vec![0x20, 0x8]);

        match path.resolve(&t) {
            Err(Error::BrokenChain { hop, .. }) => assert_eq!(hop, 1),
            other => panic!("expected a broken chain, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_nonsense_pointer() {
        let t = MockTarget::zeroed(BASE, 0x1000).with_module("game.exe", BASE, 0x1000);
        // A float read as a pointer looks like this.
        t.poke_usize(BASE + 0x100, 0xFFFF_FFFF_FFFF_0000);

        let path = PointerPath::module("game.exe", 0x100, vec![0x8, 0x4]);
        assert!(matches!(path.resolve(&t), Err(Error::BrokenChain { .. })));
    }

    #[test]
    fn missing_module_is_an_error() {
        let t = MockTarget::zeroed(BASE, 0x1000);
        let path = PointerPath::module("nothere.exe", 0x10, vec![0x0]);
        assert!(matches!(path.resolve(&t), Err(Error::ModuleNotFound(_))));
    }

    #[test]
    fn is_live_tracks_whether_the_chain_holds() {
        let t = chained();
        let path = PointerPath::module("game.exe", 0x100, vec![0x20, 0x8]);
        assert!(path.is_live(&t));

        t.poke_usize(BASE + 0x100, 0);
        assert!(!path.is_live(&t));
    }

    #[test]
    fn displays_the_way_people_write_them() {
        let path = PointerPath::module("game.exe", 0x1234, vec![0x20, -0x8]);
        assert_eq!(path.to_string(), "game.exe+0x1234 +0x20 -0x8");
    }
}
