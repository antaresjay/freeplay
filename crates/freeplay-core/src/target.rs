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

pub trait Target: Send + Sync {
    fn pid(&self) -> u32;
    fn name(&self) -> &str;

    fn arch(&self) -> Arch {
        Arch::X64
    }

    fn modules(&self) -> Result<Vec<Module>>;
    fn regions(&self) -> Result<Vec<Region>>;
    fn read_into(&self, addr: usize, buf: &mut [u8]) -> Result<()>;
    fn write_bytes(&self, addr: usize, data: &[u8]) -> Result<()>;

    fn make_writable(&self, addr: usize, len: usize) -> Result<u32>;
    fn restore_protection(&self, addr: usize, len: usize, previous: u32) -> Result<()>;

    fn allocate(&self, _size: usize, _near: Option<usize>) -> Result<usize> {
        Err(Error::Unsupported("allocating inside the target"))
    }

    fn release(&self, _addr: usize) -> Result<()> {
        Err(Error::Unsupported("releasing memory inside the target"))
    }

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
        Ok(usize::from_ne_bytes(buf))
    }

    fn module(&self, name: &str) -> Result<Module> {
        let wanted = name.to_ascii_lowercase();
        self.modules()?
            .into_iter()
            .find(|m| m.name.to_ascii_lowercase() == wanted)
            .ok_or_else(|| Error::ModuleNotFound(name.to_string()))
    }

    fn main_module(&self) -> Result<Module> {
        let name = self.name().to_string();
        self.module(&name)
    }
}
