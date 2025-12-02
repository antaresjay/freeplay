//! End to end against a real process: this one.
//!
//! Attaching to ourselves exercises OpenProcess, ReadProcessMemory,
//! WriteProcessMemory and VirtualQueryEx for real, and unlike pointing the
//! tests at Notepad it runs anywhere without anything else installed.

#![cfg(windows)]

use std::hint::black_box;

use freeplay_core::pattern::Pattern;
use freeplay_core::scanner::{self, Scope};
use freeplay_core::target::Target;
use freeplay_core::value::{Scalar, ValueKind};
use freeplay_core::windows_target::{processes, WindowsTarget};

fn attach() -> WindowsTarget {
    WindowsTarget::attach(std::process::id()).expect("attach to self")
}

#[test]
fn lists_running_processes() {
    let all = processes().expect("enumerate processes");
    assert!(all.len() > 5, "only found {} processes", all.len());
    assert!(all.iter().any(|p| p.pid == std::process::id()));
}

#[test]
fn attaches_to_this_process() {
    let target = attach();
    assert_eq!(target.pid(), std::process::id());
    assert!(target.alive());
    assert!(target.name().ends_with(".exe"), "got {:?}", target.name());
}

#[test]
fn enumerates_our_own_modules() {
    let modules = attach().modules().expect("modules");
    let names: Vec<String> = modules.iter().map(|m| m.name.to_ascii_lowercase()).collect();

    assert!(names.iter().any(|n| n == "ntdll.dll"), "no ntdll in {names:?}");
    assert!(modules.iter().all(|m| m.base != 0));
}

#[test]
fn enumerates_committed_regions() {
    let regions = attach().regions().expect("regions");
    assert!(regions.len() > 10);
    assert!(regions.iter().any(|r| r.scannable_code()), "no executable pages");
    assert!(regions.iter().any(|r| r.scannable_data()), "no private writable pages");
}

#[test]
fn reads_a_value_out_of_our_own_stack() {
    let value: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let addr = &value as *const u64 as usize;

    let got = attach().read_scalar(addr, ValueKind::U64).expect("read");
    assert_eq!(got, Scalar::U64(value));
    black_box(value);
}

#[test]
fn writes_a_value_back_into_our_own_memory() {
    let mut cell: u32 = 5;
    let addr = &mut cell as *mut u32 as usize;

    attach().write_scalar(addr, Scalar::U32(9999)).expect("write");

    assert_eq!(black_box(cell), 9999);
}

#[test]
fn reading_an_unmapped_address_fails_cleanly() {
    // Deliberately silly address, should error rather than panic or hang.
    let result = attach().read_bytes(0x10, 8);
    assert!(result.is_err());
}

#[test]
fn scans_real_memory_for_a_known_needle() {
    let needle = [0x1Bu8, 0xAD, 0xC0, 0xDE, 0xFE, 0xED, 0xFA, 0xCE, 0x13, 0x37];
    let haystack = needle.to_vec().into_boxed_slice();
    let addr = haystack.as_ptr() as usize;

    let target = attach();
    let pattern = Pattern::parse("1B AD C0 DE FE ED FA CE 13 37").unwrap();
    let hits = scanner::find_all(&target, &pattern, Scope::Data).expect("scan");

    assert!(hits.contains(&addr), "expected {addr:#x} among {} hits", hits.len());
    black_box(haystack);
}
