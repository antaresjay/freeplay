//! An in-memory target, so scanning and pointer logic can be tested without a
//! running game.

use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::region::{Protection, Region};
use crate::target::{Module, Target};

pub struct MockTarget {
    pub base: usize,
    memory: Mutex<Vec<u8>>,
    modules: Vec<Module>,
    protection: Protection,
    mapped: bool,
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
        }
    }

    pub fn zeroed(base: usize, len: usize) -> Self {
        Self::new(base, vec![0u8; len])
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
