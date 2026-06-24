use std::sync::Arc;

use freeplay_core::mock::MockTarget;
use freeplay_core::Target;
use freeplay_session::Session;
use freeplay_table::Table;

const BASE: usize = 0x0040_0000;
const SPAN: usize = 0x8000;
const HOOK: usize = BASE + 0x1000;

const ORIGINAL: [u8; 11] = [
    0x8B, 0x10, 0x8B, 0xC8, 0xFF, 0x92, 0x34, 0x02, 0x00, 0x00, 0x84,
];

const TABLE: &str = r#"
[game]
name = "The Witcher 2"
exe = "mock.exe"

[[cheat]]
id = "base"
name = "Get Witcher Base"
type = "script"
source = """
[ENABLE]
aobscanmodule(getWitcher,mock.exe,8B 10 8B C8 FF 92 34 02 00 00 84)
alloc(newgetWitcher,100,getWitcher)
label(codegetWitcher)
label(returngetWitcher)
label(baseWitcher)
registersymbol(baseWitcher)
newgetWitcher:
  mov [baseWitcher],eax
codegetWitcher:
  mov edx,[eax]
  mov ecx,eax
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
dealloc(newgetWitcher)
"""

[[cheat]]
id = "health"
name = "Current Health"
type = "freeze"
value_type = "f32"
value = 9999

[cheat.locator]
find = "symbol"
symbol = "baseWitcher"
hops = ["+0x14", "+0x8"]
"#;

fn game() -> (MockTarget, Table) {
    let target = MockTarget::zeroed(BASE, SPAN)
        .with_module("mock.exe", BASE, SPAN)
        .executable()
        .x86();
    target.poke(HOOK, &ORIGINAL);
    (target, Table::parse(TABLE).expect("table should parse"))
}

fn session() -> (Arc<MockTarget>, Session) {
    let (target, table) = game();
    let shared = Arc::new(target);
    let session = Session::new(shared.clone() as Arc<dyn Target>, table);
    (shared, session)
}

#[test]
fn arming_a_value_also_arms_the_script_that_finds_it() {
    let (_target, session) = session();
    session.arm("health").unwrap();

    assert!(session.is_armed("health"));
    assert!(
        session.is_armed("base"),
        "the script that writes baseWitcher should come with it"
    );
}

#[test]
fn the_script_engages_and_the_value_waits_for_the_pointer() {
    let (_target, session) = session();
    session.arm("health").unwrap();

    assert!(session.is_on("base"), "the script has everything it needs");
    assert!(
        !session.is_on("health"),
        "the slot is still zero, so there is nothing to freeze yet"
    );
}

#[test]
fn the_value_engages_on_its_own_once_the_game_fills_the_slot() {
    let (target, session) = session();
    session.arm("health").unwrap();

    let slot = session.symbols()["baseWitcher"] as usize;
    let player = BASE + 0x3000;
    target.poke_pointer(slot, player);
    target.poke_pointer(player + 0x14, BASE + 0x3400);

    session.reconcile();

    assert!(
        session.is_on("health"),
        "nobody should have to come back to the app for this"
    );
    let held = target.read_bytes(BASE + 0x3408, 4).unwrap();
    assert_eq!(f32::from_ne_bytes(held.try_into().unwrap()), 9999.0);
}

#[test]
fn what_is_armed_survives_the_pointer_going_away_again() {
    let (target, session) = session();
    session.arm("health").unwrap();

    let slot = session.symbols()["baseWitcher"] as usize;
    let player = BASE + 0x3000;
    target.poke_pointer(slot, player);
    target.poke_pointer(player + 0x14, BASE + 0x3400);
    session.reconcile();
    assert!(session.is_on("health"));

    session.disable("health").unwrap();
    target.poke_pointer(slot, 0);
    session.reconcile();

    assert!(!session.is_on("health"));
    assert!(
        session.is_armed("health"),
        "still wanted, just not possible"
    );
}

#[test]
fn disarming_switches_it_off_and_forgets_it() {
    let (_target, session) = session();
    session.arm("base").unwrap();
    assert!(session.is_on("base"));

    session.disarm("base").unwrap();

    assert!(!session.is_armed("base"));
    assert!(!session.is_on("base"));
}

#[test]
fn disarming_the_script_puts_the_original_bytes_back() {
    let (target, session) = session();
    session.arm("base").unwrap();
    assert_ne!(target.read_bytes(HOOK, 5).unwrap(), ORIGINAL[..5].to_vec());

    session.disarm("base").unwrap();
    assert_eq!(target.read_bytes(HOOK, 11).unwrap(), ORIGINAL.to_vec());
}

#[test]
fn arming_what_was_saved_last_time_needs_no_clicking() {
    let (target, session) = session();
    session.arm_all(&["health".to_string(), "base".to_string()]);

    let slot = session.symbols()["baseWitcher"] as usize;
    let player = BASE + 0x3000;
    target.poke_pointer(slot, player);
    target.poke_pointer(player + 0x14, BASE + 0x3400);
    session.reconcile();

    assert!(session.is_on("base"));
    assert!(session.is_on("health"));
}

#[test]
fn a_cheat_that_is_not_in_the_table_is_refused() {
    let (_target, session) = session();
    assert!(session.arm("nonsense").is_err());
}
