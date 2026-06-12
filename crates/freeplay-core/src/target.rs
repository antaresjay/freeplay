use crate::error::{Error, Result};
use crate::region::Region;
use crate::value::{Scalar, ValueKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub base: usize,
    pub size: usize,
}

impl Module {
    pub fn end(&self) -> usize {
        self.base.saturating_add(self.size)
    }

    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.end()
    }
}

/// How wide the target's pointers are.
///
/// Plenty of games worth cheating in are still 32-bit, either because they are
/// old or because they never needed the address space. A 64-bit process can
/// read and write one perfectly well, but a pointer in it is four bytes, not
/// eight, so a chain walked with the wrong width lands nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86,
    X64,
}

impl Arch {
    pub fn pointer_width(self) -> usize {
        match self {
            Arch::X86 => 4,
            Arch::X64 => 8,
        }
    }

    /// Above this an address is not a pointer, it is a float or a run of text
    /// being read as one. 32-bit Windows hands out the low 2GB by default and
    /// the low 4GB to a large address aware process, so allow the whole range
    /// rather than guess which one this is.
    pub fn ceiling(self) -> usize {
        match self {
            Arch::X86 => 0xFFFF_FFFF,
            Arch::X64 => 0x7FFF_FFFF_FFFF,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Arch::X86 => "32-bit",
            Arch::X64 => "64-bit",
        }
    }
}

/// Everything the layers above need from an operating system.
///
/// Implementors do the raw work. The scanner, pointer chains and patcher are
/// written against this and nothing else.
pub trait Target: Send + Sync {
    fn pid(&self) -> u32;
    fn name(&self) -> &str;

    /// Defaults to 64-bit, so an implementor that only ever sees one kind of
    /// process does not have to care.
    fn arch(&self) -> Arch {
        Arch::X64
    }

    fn modules(&self) -> Result<Vec<Module>>;
    fn regions(&self) -> Result<Vec<Region>>;
    fn read_into(&self, addr: usize, buf: &mut [u8]) -> Result<()>;
    fn write_bytes(&self, addr: usize, data: &[u8]) -> Result<()>;

    /// Make a span writable, returning the previous protection so it can be
    /// put back. Code pages are read-execute, so patching one means flipping
    /// this first and restoring it after.
    fn make_writable(&self, addr: usize, len: usize) -> Result<u32>;
    fn restore_protection(&self, addr: usize, len: usize, previous: u32) -> Result<()>;

    fn alive(&self) -> bool;

    fn read_bytes(&self, addr: usize, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.read_into(addr, &mut buf)?;
        Ok(buf)
    }

    fn read_scalar(&self, addr: usize, kind: ValueKind) -> Result<Scalar> {
        let bytes = self.read_bytes(addr, kind.size())?;
        kind.read(&bytes).ok_or(Error::ReadFailed {
            addr,
            len: kind.size(),
            source: std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short read"),
        })
    }

    fn write_scalar(&self, addr: usize, value: Scalar) -> Result<()> {
        self.write_bytes(addr, &value.to_bytes())
    }

    fn read_pointer(&self, addr: usize) -> Result<usize> {
        let mut buf = [0u8; 8];
        let width = self.arch().pointer_width();
        self.read_into(addr, &mut buf[..width])?;
        // The unread half stays zero, so a 32-bit pointer zero extends, which
        // is exactly what the processor does with it.
        Ok(usize::from_ne_bytes(buf))
    }

    fn module(&self, name: &str) -> Result<Module> {
        let wanted = name.to_ascii_lowercase();
        self.modules()?
            .into_iter()
            .find(|m| m.name.to_ascii_lowercase() == wanted)
            .ok_or_else(|| Error::ModuleNotFound(name.to_string()))
    }

    /// The game's own executable, which is where static addresses are anchored.
    fn main_module(&self) -> Result<Module> {
        let name = self.name().to_string();
        self.module(&name)
    }
}
