//! Overwriting instructions.
//!
//! Some cheats cannot be done by holding a number still. A mission timer is
//! rewritten by the game every frame, so freezing it produces a stuttering
//! clock rather than a stopped one. The fix is to find the instruction doing
//! the subtracting and replace it with nothing, after which there is nothing
//! left to fight.

use crate::error::{Error, Result};
use crate::target::Target;

/// x86 one byte no-op.
const NOP: u8 = 0x90;

#[derive(Debug, Clone)]
pub struct Patch {
    pub addr: usize,
    original: Vec<u8>,
    replacement: Vec<u8>,
    applied: bool,
}

impl Patch {
    /// Read what is there now so it can be put back later.
    pub fn new(target: &dyn Target, addr: usize, replacement: Vec<u8>) -> Result<Self> {
        if replacement.is_empty() {
            return Err(Error::BadPattern("replacement is empty".into()));
        }
        let original = target.read_bytes(addr, replacement.len())?;
        Ok(Self {
            addr,
            original,
            replacement,
            applied: false,
        })
    }

    /// Replace `len` bytes with no-ops.
    ///
    /// `len` has to cover whole instructions. Half an instruction leaves the
    /// processor decoding the tail of one thing as the start of another, and
    /// the game crashes.
    pub fn nop(target: &dyn Target, addr: usize, len: usize) -> Result<Self> {
        Self::new(target, addr, vec![NOP; len])
    }

    pub fn len(&self) -> usize {
        self.replacement.len()
    }

    pub fn is_empty(&self) -> bool {
        self.replacement.is_empty()
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }

    pub fn original_bytes(&self) -> &[u8] {
        &self.original
    }

    pub fn apply(&mut self, target: &dyn Target) -> Result<()> {
        if self.applied {
            return Ok(());
        }
        self.write(target, &self.replacement.clone())?;
        self.applied = true;
        Ok(())
    }

    /// Put the original instructions back.
    pub fn revert(&mut self, target: &dyn Target) -> Result<()> {
        if !self.applied {
            return Ok(());
        }
        self.write(target, &self.original.clone())?;
        self.applied = false;
        Ok(())
    }

    /// True if memory still holds what we last wrote. False means something
    /// else has been here, and reverting would put back bytes that are no
    /// longer correct.
    pub fn is_intact(&self, target: &dyn Target) -> bool {
        let expected: &[u8] = if self.applied {
            &self.replacement
        } else {
            &self.original
        };
        target
            .read_bytes(self.addr, expected.len())
            .map(|now| now == expected)
            .unwrap_or(false)
    }

    fn write(&self, target: &dyn Target, bytes: &[u8]) -> Result<()> {
        // Code pages are read and execute, so they have to be made writable
        // first and put back afterwards. Leaving a game's code writable is the
        // sort of thing an anti-cheat, or a crash dump, would notice.
        let previous = target.make_writable(self.addr, bytes.len())?;
        let result = target.write_bytes(self.addr, bytes);
        let restored = target.restore_protection(self.addr, bytes.len(), previous);
        result?;
        restored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockTarget;

    const BASE: usize = 0x4000_0000;

    /// sub dword ptr [rbx+24], eax
    const INSTRUCTION: [u8; 5] = [0x29, 0x43, 0x24, 0x48, 0x8B];

    fn target() -> MockTarget {
        let t = MockTarget::zeroed(BASE, 256).executable();
        t.poke(BASE + 16, &INSTRUCTION);
        t
    }

    #[test]
    fn nop_replaces_the_instruction() {
        let t = target();
        let mut patch = Patch::nop(&t, BASE + 16, 3).unwrap();

        patch.apply(&t).unwrap();

        assert_eq!(
            t.read_bytes(BASE + 16, 5).unwrap(),
            vec![0x90, 0x90, 0x90, 0x48, 0x8B]
        );
        assert!(patch.is_applied());
    }

    #[test]
    fn revert_puts_the_original_back() {
        let t = target();
        let mut patch = Patch::nop(&t, BASE + 16, 3).unwrap();

        patch.apply(&t).unwrap();
        patch.revert(&t).unwrap();

        assert_eq!(t.read_bytes(BASE + 16, 5).unwrap(), INSTRUCTION.to_vec());
        assert!(!patch.is_applied());
    }

    #[test]
    fn applying_twice_is_harmless() {
        let t = target();
        let mut patch = Patch::nop(&t, BASE + 16, 3).unwrap();

        patch.apply(&t).unwrap();
        patch.apply(&t).unwrap();
        patch.revert(&t).unwrap();

        // If the second apply had captured the nops as the original, this
        // would come back as 0x90s.
        assert_eq!(
            t.read_bytes(BASE + 16, 3).unwrap(),
            INSTRUCTION[..3].to_vec()
        );
    }

    #[test]
    fn reverting_before_applying_does_nothing() {
        let t = target();
        let mut patch = Patch::nop(&t, BASE + 16, 3).unwrap();
        patch.revert(&t).unwrap();
        assert_eq!(t.read_bytes(BASE + 16, 5).unwrap(), INSTRUCTION.to_vec());
    }

    #[test]
    fn arbitrary_replacement_bytes_work() {
        let t = target();
        // xor eax, eax then return, a common "always zero" stub.
        let mut patch = Patch::new(&t, BASE + 16, vec![0x31, 0xC0, 0xC3]).unwrap();

        patch.apply(&t).unwrap();
        assert_eq!(t.read_bytes(BASE + 16, 3).unwrap(), vec![0x31, 0xC0, 0xC3]);

        patch.revert(&t).unwrap();
        assert_eq!(
            t.read_bytes(BASE + 16, 3).unwrap(),
            INSTRUCTION[..3].to_vec()
        );
    }

    #[test]
    fn keeps_a_copy_of_what_was_there() {
        let t = target();
        let patch = Patch::nop(&t, BASE + 16, 5).unwrap();
        assert_eq!(patch.original_bytes(), &INSTRUCTION);
        assert_eq!(patch.len(), 5);
    }

    #[test]
    fn spots_when_something_else_has_been_here() {
        let t = target();
        let mut patch = Patch::nop(&t, BASE + 16, 3).unwrap();
        patch.apply(&t).unwrap();
        assert!(patch.is_intact(&t));

        // Another tool, or the game itself, writes over our patch.
        t.poke(BASE + 16, &[0xCC, 0xCC, 0xCC]);
        assert!(!patch.is_intact(&t));
    }

    #[test]
    fn empty_replacement_is_rejected() {
        let t = target();
        assert!(Patch::new(&t, BASE + 16, Vec::new()).is_err());
    }

    #[test]
    fn a_bad_address_fails_at_construction() {
        let t = target();
        assert!(Patch::nop(&t, BASE + 0x9999, 4).is_err());
    }
}
