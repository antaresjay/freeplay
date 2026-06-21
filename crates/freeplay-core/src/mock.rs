use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::region::{Protection, Region};
use crate::target::{Arch, Module, Target};

pub struct MockTarget {
    pub base: usize,
    memory: Mutex<Vec<u8>>,
    modules: Vec<Module>,
    protection: Protection,
    mapped: bool,
    arch: Arch,
    allocations: Mutex<Vec<(usize, usize)>>,
}

impl MockTarget {
    pub fn new(base: usize, memory: Vec<u8>) -> Self {
        Self {
            base,
            memory: Mutex::new(memory),
            modules: Vec::new(),
            protection: Protection {
                read: true,
                write: true,
                execute: false,
                guard: false,
            },
            mapped: false,
            arch: Arch::X64,
            allocations: Mutex::new(Vec::new()),
        }
    }

    pub fn zeroed(base: usize, len: usize) -> Self {
        Self::new(base, vec![0u8; len])
    }

    pub fn x86(mut self) -> Self {
        self.arch = Arch::X86;
        self
    }

    pub fn poke_pointer(&self, addr: usize, value: usize) {
        self.poke(addr, &value.to_ne_bytes()[..self.arch.pointer_width()]);
    }

    pub fn with_module(mut self, name: &str, base: usize, size: usize) -> Self {
        self.modules.push(Module {
            name: name.into(),
            base,
            size,
        });
        self
    }

    pub fn executable(mut self) -> Self {
        self.protection.execute = true;
        self.mapped = true;
        self
    }

    pub fn poke(&self, addr: usize, bytes: &[u8]) {
        let mut memory = self.memory.lock().unwrap();
        let offset = addr - self.base;
        memory[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    pub fn poke_usize(&self, addr: usize, value: usize) {
        self.poke(addr, &value.to_ne_bytes());
    }

    pub fn live_allocations(&self) -> usize {
        self.allocations.lock().unwrap().len()
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.memory.lock().unwrap().clone()
    }
}

impl Target for MockTarget {
    fn pid(&self) -> u32 {
        1234
    }

    fn name(&self) -> &str {
        "mock.exe"
    }

    fn arch(&self) -> Arch {
        self.arch
    }

    fn modules(&self) -> Result<Vec<Module>> {
        Ok(self.modules.clone())
    }

    fn regions(&self) -> Result<Vec<Region>> {
        Ok(vec![Region {
            base: self.base,
            size: self.memory.lock().unwrap().len(),
            protection: self.protection,
            mapped: self.mapped,
        }])
    }

    fn read_into(&self, addr: usize, buf: &mut [u8]) -> Result<()> {
        let memory = self.memory.lock().unwrap();
        let offset = addr
            .checked_sub(self.base)
            .filter(|o| o + buf.len() <= memory.len())
            .ok_or(Error::ReadFailed {
                addr,
                len: buf.len(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "out of range"),
            })?;
        buf.copy_from_slice(&memory[offset..offset + buf.len()]);
        Ok(())
    }

    fn write_bytes(&self, addr: usize, data: &[u8]) -> Result<()> {
        let mut memory = self.memory.lock().unwrap();
        let offset = addr
            .checked_sub(self.base)
            .filter(|o| o + data.len() <= memory.len())
            .ok_or(Error::WriteFailed {
                addr,
                len: data.len(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "out of range"),
            })?;
        memory[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    fn make_writable(&self, _addr: usize, _len: usize) -> Result<u32> {
        Ok(0x20)
    }

    fn restore_protection(&self, _addr: usize, _len: usize, _previous: u32) -> Result<()> {
        Ok(())
    }

    fn allocate(&self, size: usize, _near: Option<usize>) -> Result<usize> {
        let mut memory = self.memory.lock().unwrap();
        let addr = self.base + memory.len();
        let grown = memory.len() + size;
        memory.resize(grown, 0);
        self.allocations.lock().unwrap().push((addr, size));
        Ok(addr)
    }

    fn release(&self, addr: usize) -> Result<()> {
        self.allocations.lock().unwrap().retain(|(a, _)| *a != addr);
        Ok(())
    }

    fn alive(&self) -> bool {
        true
    }
}
