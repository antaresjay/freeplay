use std::collections::HashMap;

use freeplay_aa::{parse, Runner};
use freeplay_core::mock::MockTarget;
use freeplay_core::Target;

const BASE: usize = 0x0040_0000;
const SPAN: usize = 0x4000;
const HOOK: usize = BASE + 0x1000;

const SCRIPT: &str = r#"{
	Game	: witcher2.EXE
	Author	: aSwedishMagyar
}
[ENABLE]
aobscanmodule(getWitcher,witcher2.EXE,8B 10 8B C8 FF 92 34 02 00 00 84)
registersymbol(getWitcher)
registersymbol(baseWitcher)
alloc(newgetWitcher,100,getWitcher)
label(codegetWitcher)
label(returngetWitcher)
label(baseWitcher)
newgetWitcher:
  mov [baseWitcher],eax
codegetWitcher:
  mov edx,[eax]
  mov ecx,eax
  call dword ptr [edx+00000234]
  jmp returngetWitcher
baseWitcher:
  dd 0
getWitcher:
  jmp newgetWitcher
  nop 5
returngetWitcher:

[DISABLE]
getWitcher:
  db 8B 10 8B C8 FF 92 34 02 00 00

unregistersymbol(getWitcher)
unregistersymbol(baseWitcher)
dealloc(newgetWitcher)
"#;

const ORIGINAL: [u8; 11] = [
    0x8B, 0x10, 0x8B, 0xC8, 0xFF, 0x92, 0x34, 0x02, 0x00, 0x00, 0x84,
];

fn game() -> MockTarget {
    let target = MockTarget::zeroed(BASE, SPAN)
        .with_module("witcher2.EXE", BASE, SPAN)
        .executable()
        .x86();
    target.poke(HOOK, &ORIGINAL);
    target
}

fn read(target: &MockTarget, addr: usize, len: usize) -> Vec<u8> {
    target.read_bytes(addr, len).unwrap()
}

#[test]
fn the_hook_replaces_the_original_instructions_with_a_jump() {
    let target = game();
    let script = parse(SCRIPT).unwrap();
    let engaged = Runner::new(&target)
        .enable(&script, &HashMap::new())
        .expect("should engage");

    let cave = engaged.symbols["newgetWitcher"] as usize;
    let patched = read(&target, HOOK, 10);

    assert_eq!(patched[0], 0xE9);
    let delta = i32::from_le_bytes(patched[1..5].try_into().unwrap()) as i64;
    assert_eq!(HOOK as i64 + 5 + delta, cave as i64);
    assert_eq!(&patched[5..10], &[0x90; 5]);
}

#[test]
fn the_byte_after_the_hook_is_left_alone() {
    let target = game();
    let script = parse(SCRIPT).unwrap();
    Runner::new(&target)
        .enable(&script, &HashMap::new())
        .unwrap();

    assert_eq!(read(&target, HOOK + 10, 1), vec![0x84]);
}

#[test]
fn the_cave_captures_the_register_then_runs_the_original_code() {
    let target = game();
    let script = parse(SCRIPT).unwrap();
    let engaged = Runner::new(&target)
        .enable(&script, &HashMap::new())
        .unwrap();

    let cave = engaged.symbols["newgetWitcher"] as usize;
    let slot = engaged.symbols["baseWitcher"] as usize;

    let written = read(&target, cave, 6);
    assert_eq!(written[0], 0x89, "mov [addr],eax");
    assert_eq!(written[1], 0x05);
    assert_eq!(
        u32::from_le_bytes(written[2..6].try_into().unwrap()),
        slot as u32
    );

    let body = read(&target, cave + 6, 10);
    assert_eq!(&body[0..2], &[0x8B, 0x10], "mov edx,[eax]");
    assert_eq!(&body[2..4], &[0x8B, 0xC8], "mov ecx,eax");
    assert_eq!(&body[4..10], &[0xFF, 0x92, 0x34, 0x02, 0x00, 0x00]);
}

#[test]
fn the_cave_jumps_back_to_just_after_the_hook() {
    let target = game();
    let script = parse(SCRIPT).unwrap();
    let engaged = Runner::new(&target)
        .enable(&script, &HashMap::new())
        .unwrap();

    let cave = engaged.symbols["newgetWitcher"] as usize;
    let jump = cave + 16;
    let bytes = read(&target, jump, 5);
    assert_eq!(bytes[0], 0xE9);

    let delta = i32::from_le_bytes(bytes[1..5].try_into().unwrap()) as i64;
    assert_eq!(jump as i64 + 5 + delta, (HOOK + 10) as i64);
}

#[test]
fn the_slot_the_values_hang_off_gets_an_address() {
    let target = game();
    let script = parse(SCRIPT).unwrap();
    let engaged = Runner::new(&target)
        .enable(&script, &HashMap::new())
        .unwrap();

    let slot = engaged.symbols["baseWitcher"];
    let cave = engaged.symbols["newgetWitcher"];
    assert!(slot > cave, "the slot lives inside the cave");
    assert_eq!(read(&target, slot as usize, 4), vec![0, 0, 0, 0]);
}

#[test]
fn disabling_puts_the_original_bytes_back() {
    let target = game();
    let script = parse(SCRIPT).unwrap();
    let runner = Runner::new(&target);
    let engaged = runner.enable(&script, &HashMap::new()).unwrap();

    assert_eq!(target.live_allocations(), 1);
    runner.disable(&script, &engaged).unwrap();

    assert_eq!(read(&target, HOOK, 11), ORIGINAL.to_vec());
    assert_eq!(target.live_allocations(), 0);
}

#[test]
fn a_build_without_the_signature_says_so_rather_than_writing_anywhere() {
    let target = MockTarget::zeroed(BASE, SPAN)
        .with_module("witcher2.EXE", BASE, SPAN)
        .executable()
        .x86();

    let script = parse(SCRIPT).unwrap();
    let outcome = Runner::new(&target).enable(&script, &HashMap::new());

    assert!(outcome.is_err());
    assert_eq!(read(&target, HOOK, 11), vec![0; 11]);
}
