//! Windows backend. Every OS call in Freeplay is in this file.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{
    VirtualProtectEx, VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_IMAGE, MEM_MAPPED,
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, IsWow64Process, OpenProcess, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

use crate::error::{Error, Result};
use crate::guard;
use crate::region::{Protection, Region};
use crate::target::{Module, Target};

const STILL_RUNNING: u32 = 259;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

pub struct WindowsTarget {
    handle: HANDLE,
    pid: u32,
    name: String,
}

// The handle is only ever passed to ReadProcessMemory, WriteProcessMemory and
// VirtualQueryEx, all of which are thread safe against the same handle. The
// scanner relies on this to sweep regions in parallel.
unsafe impl Send for WindowsTarget {}
unsafe impl Sync for WindowsTarget {}

struct Snapshot(HANDLE);

impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn os_error(err: windows::core::Error) -> io::Error {
    io::Error::other(err)
}

pub fn processes() -> Result<Vec<ProcessInfo>> {
    let snapshot = Snapshot(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| Error::Io(os_error(e)))?,
    );

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut out = Vec::new();
    unsafe {
        if Process32FirstW(snapshot.0, &mut entry).is_err() {
            return Ok(out);
        }
        loop {
            out.push(ProcessInfo {
                pid: entry.th32ProcessID,
                name: wide_to_string(&entry.szExeFile),
            });
            if Process32NextW(snapshot.0, &mut entry).is_err() {
                break;
            }
        }
    }
    Ok(out)
}

impl WindowsTarget {
    pub fn attach_by_name(name: &str) -> Result<Self> {
        let wanted = name.to_ascii_lowercase();
        let found = processes()?
            .into_iter()
            .find(|p| p.name.to_ascii_lowercase() == wanted)
            .ok_or_else(|| Error::ProcessNotFound(name.to_string()))?;
        Self::attach(found.pid)
    }

    pub fn attach(pid: u32) -> Result<Self> {
        let name = processes()?
            .into_iter()
            .find(|p| p.pid == pid)
            .map(|p| p.name)
            .unwrap_or_else(|| format!("pid {pid}"));

        if guard::is_protected_process(&name) {
            return Err(Error::Protected {
                process: name,
                guard: "an anti-cheat service",
            });
        }

        let handle = unsafe {
            OpenProcess(
                PROCESS_VM_READ
                    | PROCESS_VM_WRITE
                    | PROCESS_VM_OPERATION
                    | PROCESS_QUERY_INFORMATION,
                false,
                pid,
            )
        }
        .map_err(|e| Error::OpenFailed {
            pid,
            source: os_error(e),
        })?;

        let target = Self { handle, pid, name };

        // Refuse before the handle is used for anything else.
        let modules = target.modules()?;
        if let Some(product) = guard::inspect_modules(&modules) {
            return Err(Error::Protected {
                process: target.name.clone(),
                guard: product,
            });
        }

        if target.is_wow64()? {
            return Err(Error::ArchMismatch);
        }

        Ok(target)
    }

    fn is_wow64(&self) -> Result<bool> {
        let mut wow64 = windows::core::BOOL(0);
        unsafe { IsWow64Process(self.handle, &mut wow64) }.map_err(|e| Error::Io(os_error(e)))?;
        Ok(wow64.as_bool())
    }
}

impl Drop for WindowsTarget {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn decode_protection(flags: PAGE_PROTECTION_FLAGS) -> Protection {
    const NOACCESS: u32 = 0x01;
    const READONLY: u32 = 0x02;
    const READWRITE: u32 = 0x04;
    const WRITECOPY: u32 = 0x08;
    const EXECUTE: u32 = 0x10;
    const EXECUTE_READ: u32 = 0x20;
    const EXECUTE_READWRITE: u32 = 0x40;
    const EXECUTE_WRITECOPY: u32 = 0x80;
    const GUARD: u32 = 0x100;

    let raw = flags.0;
    // The low byte is the protection, the rest are modifiers like PAGE_GUARD.
    let base = raw & 0xFF;

    let (read, write, execute) = match base {
        NOACCESS => (false, false, false),
        READONLY => (true, false, false),
        READWRITE | WRITECOPY => (true, true, false),
        EXECUTE => (false, false, true),
        EXECUTE_READ => (true, false, true),
        EXECUTE_READWRITE | EXECUTE_WRITECOPY => (true, true, true),
        _ => (false, false, false),
    };

    Protection {
        read,
        write,
        execute,
        guard: raw & GUARD != 0,
    }
}

impl Target for WindowsTarget {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn modules(&self) -> Result<Vec<Module>> {
        // A process that is still starting up returns ERROR_BAD_LENGTH here,
        // and the documented fix is simply to ask again.
        let mut last = None;
        for _ in 0..8 {
            match unsafe {
                CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.pid)
            } {
                Ok(raw) => {
                    let snapshot = Snapshot(raw);
                    let mut entry = MODULEENTRY32W {
                        dwSize: size_of::<MODULEENTRY32W>() as u32,
                        ..Default::default()
                    };
                    let mut out = Vec::new();
                    unsafe {
                        if Module32FirstW(snapshot.0, &mut entry).is_ok() {
                            loop {
                                out.push(Module {
                                    name: wide_to_string(&entry.szModule),
                                    base: entry.modBaseAddr as usize,
                                    size: entry.modBaseSize as usize,
                                });
                                if Module32NextW(snapshot.0, &mut entry).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    return Ok(out);
                }
                Err(e) => last = Some(e),
            }
            std::thread::yield_now();
        }
        Err(Error::Io(os_error(last.expect("loop runs at least once"))))
    }

    fn regions(&self) -> Result<Vec<Region>> {
        let mut out = Vec::new();
        let mut addr: usize = 0;

        loop {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            let written = unsafe {
                VirtualQueryEx(
                    self.handle,
                    Some(addr as *const c_void),
                    &mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if written == 0 {
                break;
            }

            let base = info.BaseAddress as usize;
            if info.State == MEM_COMMIT {
                out.push(Region {
                    base,
                    size: info.RegionSize,
                    protection: decode_protection(info.Protect),
                    mapped: info.Type == MEM_IMAGE || info.Type == MEM_MAPPED,
                });
            }

            let next = base.saturating_add(info.RegionSize);
            if next <= addr {
                break;
            }
            addr = next;
        }

        Ok(out)
    }

    fn read_into(&self, addr: usize, buf: &mut [u8]) -> Result<()> {
        let mut read = 0usize;
        unsafe {
            ReadProcessMemory(
                self.handle,
                addr as *const c_void,
                buf.as_mut_ptr().cast::<c_void>(),
                buf.len(),
                Some(&mut read),
            )
        }
        .map_err(|e| Error::ReadFailed {
            addr,
            len: buf.len(),
            source: os_error(e),
        })?;

        if read != buf.len() {
            return Err(Error::ReadFailed {
                addr,
                len: buf.len(),
                source: io::Error::new(io::ErrorKind::UnexpectedEof, format!("read {read} bytes")),
            });
        }
        Ok(())
    }

    fn write_bytes(&self, addr: usize, data: &[u8]) -> Result<()> {
        let mut written = 0usize;
        unsafe {
            WriteProcessMemory(
                self.handle,
                addr as *const c_void,
                data.as_ptr().cast::<c_void>(),
                data.len(),
                Some(&mut written),
            )
        }
        .map_err(|e| Error::WriteFailed {
            addr,
            len: data.len(),
            source: os_error(e),
        })?;

        if written != data.len() {
            return Err(Error::WriteFailed {
                addr,
                len: data.len(),
                source: io::Error::new(io::ErrorKind::WriteZero, format!("wrote {written} bytes")),
            });
        }
        Ok(())
    }

    fn make_writable(&self, addr: usize, len: usize) -> Result<u32> {
        let mut previous = PAGE_PROTECTION_FLAGS(0);
        unsafe {
            VirtualProtectEx(
                self.handle,
                addr as *const c_void,
                len,
                PAGE_EXECUTE_READWRITE,
                &mut previous,
            )
        }
        .map_err(|e| Error::WriteFailed {
            addr,
            len,
            source: os_error(e),
        })?;
        Ok(previous.0)
    }

    fn restore_protection(&self, addr: usize, len: usize, previous: u32) -> Result<()> {
        let mut ignored = PAGE_PROTECTION_FLAGS(0);
        unsafe {
            VirtualProtectEx(
                self.handle,
                addr as *const c_void,
                len,
                PAGE_PROTECTION_FLAGS(previous),
                &mut ignored,
            )
        }
        .map_err(|e| Error::WriteFailed {
            addr,
            len,
            source: os_error(e),
        })?;
        Ok(())
    }

    fn alive(&self) -> bool {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.handle, &mut code) }.is_ok() && code == STILL_RUNNING
    }
}
