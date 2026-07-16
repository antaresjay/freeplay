//! End to end against a real process: this one.
//!
//! Attaching to ourselves exercises OpenProcess, ReadProcessMemory,
//! WriteProcessMemory and VirtualQueryEx for real, and unlike pointing the
//! tests at Notepad it runs anywhere without anything else installed.

#![cfg(windows)]

use std::hint::black_box;

use freeplay_core::pattern::Pattern;
use freeplay_core::scanner::{self, Scope};
use freeplay_core::search::{Filter, Search};
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
    let names: Vec<String> = modules
        .iter()
        .map(|m| m.name.to_ascii_lowercase())
        .collect();

    assert!(
        names.iter().any(|n| n == "ntdll.dll"),
        "no ntdll in {names:?}"
    );
    assert!(modules.iter().all(|m| m.base != 0));
}

#[test]
fn enumerates_committed_regions() {
    let regions = attach().regions().expect("regions");
    assert!(regions.len() > 10);
    assert!(
        regions.iter().any(|r| r.scannable_code()),
        "no executable pages"
    );
    assert!(
        regions.iter().any(|r| r.scannable_data()),
        "no private writable pages"
    );
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

    attach()
        .write_scalar(addr, Scalar::U32(9999))
        .expect("write");

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

    // The other tests in this binary run on their own threads and allocate
    // while this scan is walking the heap, so a region can be split or freed
    // between being listed and being read. Any real scan of a running game has
    // the same problem, which is why every guide tells you to pause the game
    // first. One retry is enough to ride out the churn.
    let mut hits = Vec::new();
    for _ in 0..3 {
        hits = scanner::find_all(&target, &pattern, Scope::Data).expect("scan");
        if hits.contains(&addr) {
            break;
        }
    }

    assert!(
        hits.contains(&addr),
        "expected {addr:#x} among {} hits",
        hits.len()
    );
    black_box(haystack);
}

/* the whole point of the value finder, against memory that is really moving.
search.rs tests the narrowing on its own and the scan above proves the read
path, but nobody had put the two together: find a number, change it, search
again, and end up holding the address of the thing that changed. */
#[test]
fn narrows_down_to_the_value_that_actually_moved() {
    /* a couple of megabytes, which windows hands out as its own mapping
    rather than carving off the small block heap. the other tests in this
    binary allocate while this one is scanning, and a region that gets split
    between rounds fails the re-read, which drops every candidate in it */
    let mut buffer = vec![0i32; 512 * 1024];
    buffer[4096] = 1_234_567;
    let addr = &buffer[4096] as *const i32 as usize;

    let target = attach();

    let mut search = Search::first(
        &target,
        ValueKind::I32,
        Filter::Exact(Scalar::I32(1_234_567)),
    )
    .expect("first scan");

    let found = search.len();
    assert!(found > 0, "the first scan found nothing at all");
    assert!(
        search.results(usize::MAX).iter().any(|c| c.addr == addr),
        "the first scan missed our own value at {addr:#x}, {found} candidates"
    );

    // this is the part a player does in game
    buffer[4096] = 7_654_321;

    search
        .next(&target, Filter::Exact(Scalar::I32(7_654_321)))
        .expect("second scan");

    let left = search.results(usize::MAX);
    assert!(
        left.iter().any(|c| c.addr == addr),
        "narrowing threw away the address that actually changed"
    );
    assert!(
        left.len() <= found,
        "narrowing left more than it started with, {} then {}",
        found,
        left.len()
    );
    assert_eq!(search.rounds(), 2);

    // and the address it kept is one we can write through
    target
        .write_scalar(addr, Scalar::I32(99))
        .expect("write to the address the search found");
    assert_eq!(buffer[4096], 99);

    black_box(buffer);
}
