//! An in-memory target, so scanning and pointer logic can be tested without a
//! running game.

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
        }
    }

    pub fn zeroed(base: usize, len: usize) -> Self {
        Self::new(base, vec![0u8; len])
    }

    /// Pretend to be a 32-bit game, so pointer chains can be tested at the
    /// width they are actually walked at.
    pub fn x86(mut self) -> Self {
        self.arch = Arch::X86;
        self
    }

    /// Write a pointer the way the target would store one, which is four bytes
    /// on a 32-bit process and eight on a 64-bit one.
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

    fn alive(&self) -> bool {
        true
    }
}
