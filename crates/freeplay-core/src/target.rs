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

/// Everything the layers above need from an operating system.
///
/// Implementors do the raw work. The scanner, pointer chains and patcher are
/// written against this and nothing else.
pub trait Target: Send + Sync {
    fn pid(&self) -> u32;
    fn name(&self) -> &str;
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
        self.read_into(addr, &mut buf)?;
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
