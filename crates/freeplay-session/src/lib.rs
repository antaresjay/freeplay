use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use freeplay_aa::{Runner, Script};
use freeplay_core::patch::Patch;
use freeplay_core::target::Target;
use freeplay_core::value::Scalar;
use freeplay_table::resolve::{self, State, Symbols};
use freeplay_table::schema::{Action, Cheat, Locator};
use freeplay_table::Table;

pub const TICK: Duration = Duration::from_millis(30);

const RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no cheat called {0}")]
    NoSuchCheat(String),

    #[error("{name} is not available: {reason}")]
    NotReady { name: String, reason: String },

    #[error("{name} patches the same code as {clashes_with}, so only one of them can be on")]
    Clash { name: String, clashes_with: String },

    #[error(transparent)]
    Core(#[from] freeplay_core::Error),

    #[error("{0}")]
    Table(String),

    #[error("{name}: {source}")]
    Script {
        name: String,
        #[source]
        source: freeplay_aa::AaError,
    },
}

enum Engaged {
    Freeze {
        addr: usize,
        value: Scalar,
    },
    Patched(Patch),
    Injected {
        script: Box<Script>,
        engaged: Box<freeplay_aa::Engaged>,
    },
    Done,
}

pub struct Session {
    target: Arc<dyn Target>,
    table: Table,
    engaged: Arc<Mutex<HashMap<String, Engaged>>>,
    symbols: Arc<Mutex<Symbols>>,
    armed: Arc<Mutex<HashSet<String>>>,
    // numbers the player typed, over whatever the table suggests
    chosen: Arc<Mutex<HashMap<String, Scalar>>>,
    // when we last failed to turn a cheat on, and why. the reason is kept
    // because a script that will not inject looks exactly like one that is
    // waiting for the game, and the difference is the whole answer
    tried: Arc<Mutex<HashMap<String, (Instant, String)>>>,
    // whether anything was ever switched on here. asking somebody whether a
    // table worked when they never turned a cheat on is asking them to guess
    used: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    pub fn new(target: Arc<dyn Target>, table: Table) -> Self {
        Self {
            target,
            table,
            engaged: Arc::new(Mutex::new(HashMap::new())),
            symbols: Arc::new(Mutex::new(Symbols::new())),
            armed: Arc::new(Mutex::new(HashSet::new())),
            chosen: Arc::new(Mutex::new(HashMap::new())),
            tried: Arc::new(Mutex::new(HashMap::new())),
            used: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }

    pub fn table(&self) -> &Table {
        &self.table
    }

    pub fn target(&self) -> &Arc<dyn Target> {
        &self.target
    }

    pub fn symbols(&self) -> Symbols {
        self.symbols.lock().unwrap().clone()
    }

    pub fn survey(&self) -> Vec<(String, State)> {
        let symbols = self.symbols();
        self.table
            .cheats
            .iter()
            .map(|cheat| (cheat.id.clone(), self.state_of(cheat, &symbols)))
            .collect()
    }

    pub fn state_of(&self, cheat: &Cheat, symbols: &Symbols) -> State {
        match &cheat.locator {
            Some(locator) => resolve::evaluate_with(self.target.as_ref(), locator, symbols),
            None if cheat.action.is_script() => State::Ready { addr: 0 },
            None => State::Broken {
                reason: "this cheat has no address".into(),
            },
        }
    }

    pub fn is_on(&self, id: &str) -> bool {
        self.engaged.lock().unwrap().contains_key(id)
    }

    pub fn is_armed(&self, id: &str) -> bool {
        self.armed.lock().unwrap().contains(id)
    }

    pub fn armed(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.armed.lock().unwrap().iter().cloned().collect();
        ids.sort();
        ids
    }

    // was anything ever armed in this session, this launch or a previous one
    pub fn used(&self) -> bool {
        self.used.load(Ordering::Relaxed)
    }

    pub fn arm(&self, id: &str) -> Result<(), Error> {
        if self.table.cheat(id).is_none() {
            return Err(Error::NoSuchCheat(id.to_string()));
        }
        self.used.store(true, Ordering::Relaxed);

        let mut wanted = vec![id.to_string()];
        if let Some(script) = self.provider_for(id) {
            wanted.push(script);
        }

        {
            let mut armed = self.armed.lock().unwrap();
            let mut tried = self.tried.lock().unwrap();
            for name in &wanted {
                armed.insert(name.clone());
                tried.remove(name);
            }
        }

        self.reconcile();
        Ok(())
    }

    pub fn disarm(&self, id: &str) -> Result<(), Error> {
        self.armed.lock().unwrap().remove(id);
        self.tried.lock().unwrap().remove(id);
        self.disable(id)
    }

    pub fn arm_all(&self, ids: &[String]) {
        let mut armed = self.armed.lock().unwrap();
        for id in ids {
            if self.table.cheat(id).is_some() {
                armed.insert(id.clone());
                self.used.store(true, Ordering::Relaxed);
            }
        }
        drop(armed);
        self.reconcile();
    }

    fn provider_for(&self, id: &str) -> Option<String> {
        let cheat = self.table.cheat(id)?;
        let Some(Locator::Symbol { symbol, .. }) = &cheat.locator else {
            return None;
        };

        self.table
            .cheats
            .iter()
            .find(|other| match &other.action {
                Action::Script { source } => freeplay_aa::parse(source)
                    .map(|script| freeplay_aa::symbols_defined(&script).contains(symbol))
                    .unwrap_or(false),
                _ => false,
            })
            .map(|other| other.id.clone())
    }

    pub fn reconcile(&self) {
        if !self.target.alive() {
            return;
        }

        let wanted = self.armed();
        if wanted.is_empty() {
            return;
        }

        let mut order: Vec<&Cheat> = self
            .table
            .cheats
            .iter()
            .filter(|c| wanted.contains(&c.id) && !self.is_on(&c.id))
            .collect();
        order.sort_by_key(|c| !c.action.is_script());

        for cheat in order {
            {
                let tried = self.tried.lock().unwrap();
                if tried
                    .get(&cheat.id)
                    .is_some_and(|(at, _)| at.elapsed() < RETRY)
                {
                    continue;
                }
            }

            if !self.state_of(cheat, &self.symbols()).is_ready() {
                continue;
            }

            match self.enable(&cheat.id) {
                Ok(()) => {
                    self.tried.lock().unwrap().remove(&cheat.id);
                }
                Err(e) => {
                    tracing::debug!("{} not on yet: {e}", cheat.name);
                    self.tried
                        .lock()
                        .unwrap()
                        .insert(cheat.id.clone(), (Instant::now(), e.to_string()));
                }
            }
        }
    }

    // why the last attempt to turn this on did not take. a script whose scan
    // finds nothing sits there saying it is waiting for the game otherwise,
    // and nobody can tell that apart from it genuinely waiting
    pub fn why_not(&self, id: &str) -> Option<String> {
        self.tried
            .lock()
            .unwrap()
            .get(id)
            .map(|(_, why)| why.clone())
    }

    pub fn active_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.engaged.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn enable(&self, id: &str) -> Result<(), Error> {
        let cheat = self
            .table
            .cheat(id)
            .ok_or_else(|| Error::NoSuchCheat(id.to_string()))?;

        if self.is_on(id) {
            return Ok(());
        }

        let addr = match self.state_of(cheat, &self.symbols()) {
            State::Ready { addr } => addr,
            State::Unavailable { reason } | State::Broken { reason } => {
                return Err(Error::NotReady {
                    name: cheat.name.clone(),
                    reason,
                })
            }
        };

        let engaged = self.engage(cheat, addr)?;
        self.engaged.lock().unwrap().insert(id.to_string(), engaged);
        Ok(())
    }

    // what the cheat will write. the typed number wins, then whatever the
    // table suggested, then whatever the game is holding right now
    pub fn value_for(&self, id: &str) -> Option<Scalar> {
        let cheat = self.table.cheat(id)?;
        let kind = cheat.action.kind()?;

        if let Some(picked) = self.chosen.lock().unwrap().get(id) {
            return Some(*picked);
        }
        if let Some(suggested) = cheat.action.default_value() {
            return Some(suggested.to_scalar(kind));
        }
        self.live_value(id)
    }

    // what is at the address at this instant, for showing next to the box
    pub fn live_value(&self, id: &str) -> Option<Scalar> {
        let cheat = self.table.cheat(id)?;
        let kind = cheat.action.kind()?;
        let State::Ready { addr } = self.state_of(cheat, &self.symbols()) else {
            return None;
        };
        self.target.read_scalar(addr, kind).ok()
    }

    pub fn choose(&self, id: &str, text: &str) -> Result<Scalar, Error> {
        let cheat = self
            .table
            .cheat(id)
            .ok_or_else(|| Error::NoSuchCheat(id.to_string()))?;
        let kind = cheat
            .action
            .kind()
            .ok_or_else(|| Error::Table(format!("{} does not take a number", cheat.name)))?;
        let scalar = kind
            .parse(text.trim())
            .ok_or_else(|| Error::Table(format!("{text:?} is not a {kind}")))?;

        self.chosen.lock().unwrap().insert(id.to_string(), scalar);

        // already running, so put the new number in without a round trip
        // through disable and enable
        let mut engaged = self.engaged.lock().unwrap();
        if let Some(Engaged::Freeze { addr, value }) = engaged.get_mut(id) {
            *value = scalar;
            let addr = *addr;
            drop(engaged);
            self.target.write_scalar(addr, scalar)?;
        }
        Ok(scalar)
    }

    /* the name of an already running script whose hook sites this one would
    write over. allocations are fresh pages every time so they never clash,
    only the patches back into the game's own code can */
    fn overlaps(&self, fresh: &freeplay_aa::Engaged) -> Option<String> {
        let taken = self.engaged.lock().unwrap();
        let mine: Vec<(usize, usize)> = fresh
            .restores
            .iter()
            .map(|r| (r.addr, r.addr + r.original.len()))
            .collect();

        for (id, held) in taken.iter() {
            let Engaged::Injected { engaged, .. } = held else {
                continue;
            };
            for theirs in &engaged.restores {
                let range = (theirs.addr, theirs.addr + theirs.original.len());
                if mine.iter().any(|m| m.0 < range.1 && range.0 < m.1) {
                    return Some(self.name_of(id));
                }
            }
        }
        None
    }

    fn name_of(&self, id: &str) -> String {
        self.table
            .cheats
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    fn engage(&self, cheat: &Cheat, addr: usize) -> Result<Engaged, Error> {
        match &cheat.action {
            Action::Value { lock, .. } => {
                let scalar = self.value_for(&cheat.id).ok_or_else(|| Error::NotReady {
                    name: cheat.name.clone(),
                    reason: "nothing to write yet".into(),
                })?;
                self.target.write_scalar(addr, scalar)?;
                Ok(if *lock {
                    Engaged::Freeze {
                        addr,
                        value: scalar,
                    }
                } else {
                    Engaged::Done
                })
            }
            Action::Set { kind, value } => {
                let scalar = value.to_scalar(kind.0);
                self.target.write_scalar(addr, scalar)?;
                Ok(Engaged::Done)
            }
            Action::Freeze { kind, value } => {
                let scalar = value.to_scalar(kind.0);
                self.target.write_scalar(addr, scalar)?;
                Ok(Engaged::Freeze {
                    addr,
                    value: scalar,
                })
            }
            Action::Nop { length } => {
                let mut patch = Patch::nop(self.target.as_ref(), addr, *length)?;
                patch.apply(self.target.as_ref())?;
                Ok(Engaged::Patched(patch))
            }
            Action::Bytes { replacement } => {
                let bytes = resolve::parse_bytes(replacement).map_err(Error::Table)?;
                let mut patch = Patch::new(self.target.as_ref(), addr, bytes)?;
                patch.apply(self.target.as_ref())?;
                Ok(Engaged::Patched(patch))
            }
            Action::Script { source } => {
                let script = freeplay_aa::parse(source).map_err(|e| Error::Script {
                    name: cheat.name.clone(),
                    source: e,
                })?;
                let known = self.symbols();
                let engaged = Runner::new(self.target.as_ref())
                    .enable(&script, &known)
                    .map_err(|e| Error::Script {
                        name: cheat.name.clone(),
                        source: e,
                    })?;

                /* two tables can hook the same instruction. both write a jump
                there, and turning the first one off puts back bytes that
                now belong to the second, which corrupts the game. worth
                undoing this one and saying so */
                if let Some(other) = self.overlaps(&engaged) {
                    let hold = engaged;
                    let _ = Runner::new(self.target.as_ref()).disable(&script, &hold);
                    return Err(Error::Clash {
                        name: cheat.name.clone(),
                        clashes_with: other,
                    });
                }

                self.symbols.lock().unwrap().extend(
                    engaged
                        .symbols
                        .iter()
                        .map(|(name, addr)| (name.clone(), *addr)),
                );

                Ok(Engaged::Injected {
                    script: Box::new(script),
                    engaged: Box::new(engaged),
                })
            }
        }
    }

    pub fn disable(&self, id: &str) -> Result<(), Error> {
        let Some(mut engaged) = self.engaged.lock().unwrap().remove(id) else {
            return Ok(());
        };
        match &mut engaged {
            Engaged::Patched(patch) => patch.revert(self.target.as_ref())?,
            Engaged::Injected { script, engaged } => {
                Runner::new(self.target.as_ref())
                    .disable(script, engaged)
                    .map_err(|e| Error::Script {
                        name: id.to_string(),
                        source: e,
                    })?;
                let mut symbols = self.symbols.lock().unwrap();
                for name in engaged.symbols.keys() {
                    symbols.remove(name);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn disarm_all(&self) {
        self.armed.lock().unwrap().clear();
        self.tried.lock().unwrap().clear();
        self.disable_all();
    }

    pub fn disable_all(&self) {
        let ids = self.active_ids();
        for id in ids {
            if let Err(e) = self.disable(&id) {
                tracing::warn!("could not turn off {id}: {e}");
            }
        }
    }

    pub fn tick(&self) {
        let engaged = self.engaged.lock().unwrap();
        for (id, item) in engaged.iter() {
            if let Engaged::Freeze { addr, value } = item {
                if let Err(e) = self.target.write_scalar(*addr, *value) {
                    tracing::debug!("freeze {id} failed: {e}");
                }
            }
        }
    }

    pub fn start(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let target = Arc::clone(&self.target);
        let engaged = Arc::clone(&self.engaged);
        let stop = Arc::clone(&self.stop);

        self.worker = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if !target.alive() {
                    break;
                }
                {
                    let held = engaged.lock().unwrap();
                    for item in held.values() {
                        if let Engaged::Freeze { addr, value } = item {
                            let _ = target.write_scalar(*addr, *value);
                        }
                    }
                }
                std::thread::sleep(TICK);
            }
        }));
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        self.disable_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeplay_core::mock::MockTarget;
    use freeplay_core::value::ValueKind;

    const BASE: usize = 0x6000_0000;

    fn table() -> Table {
        Table::parse(
            r#"
            [game]
            name = "Test"
            exe = "mock.exe"

            [[cheat]]
            id = "health"
            name = "Infinite Health"
            type = "freeze"
            value_type = "i32"
            value = 999
            [cheat.locator]
            find = "static"
            module = "mock.exe"
            offset = "0x40"

            [[cheat]]
            id = "money"
            name = "Set Money"
            type = "set"
            value_type = "i32"
            value = 5000
            [cheat.locator]
            find = "static"
            module = "mock.exe"
            offset = "0x80"

            [[cheat]]
            id = "timer"
            name = "Freeze Timer"
            type = "nop"
            length = 3
            [cheat.locator]
            find = "static"
            module = "mock.exe"
            offset = "0xC0"

            [[cheat]]
            id = "weight"
            name = "Carry Weight"
            type = "value"
            value_type = "i32"
            [cheat.locator]
            find = "static"
            module = "mock.exe"
            offset = "0x100"

            [[cheat]]
            id = "speed"
            name = "Game Speed"
            type = "value"
            value_type = "f32"
            value = 1.0
            lock = false
            [cheat.locator]
            find = "static"
            module = "mock.exe"
            offset = "0x140"

            [[cheat]]
            id = "orphan"
            name = "Broken One"
            type = "nop"
            length = 2
            [cheat.locator]
            find = "static"
            module = "missing.exe"
            offset = 0
            "#,
        )
        .expect("table parses")
    }

    fn session() -> (Arc<MockTarget>, Session) {
        let mock = Arc::new(MockTarget::zeroed(BASE, 0x400).with_module("mock.exe", BASE, 0x400));
        mock.poke(BASE + 0xC0, &[0x29, 0x43, 0x24]);
        let target: Arc<dyn Target> = Arc::clone(&mock) as Arc<dyn Target>;
        (mock, Session::new(target, table()))
    }

    #[test]
    fn freeze_writes_immediately() {
        let (mock, s) = session();
        s.enable("health").unwrap();
        assert_eq!(
            mock.read_scalar(BASE + 0x40, ValueKind::I32).unwrap(),
            Scalar::I32(999)
        );
        assert!(s.is_on("health"));
    }

    #[test]
    fn a_tick_puts_the_value_back() {
        let (mock, s) = session();
        s.enable("health").unwrap();

        mock.poke(BASE + 0x40, &7i32.to_ne_bytes());
        s.tick();

        assert_eq!(
            mock.read_scalar(BASE + 0x40, ValueKind::I32).unwrap(),
            Scalar::I32(999)
        );
    }

    #[test]
    fn set_writes_once_and_does_not_hold() {
        let (mock, s) = session();
        s.enable("money").unwrap();
        mock.poke(BASE + 0x80, &3i32.to_ne_bytes());
        s.tick();

        assert_eq!(
            mock.read_scalar(BASE + 0x80, ValueKind::I32).unwrap(),
            Scalar::I32(3)
        );
    }

    #[test]
    fn nop_patches_and_reverts() {
        let (mock, s) = session();
        s.enable("timer").unwrap();
        assert_eq!(
            mock.read_bytes(BASE + 0xC0, 3).unwrap(),
            vec![0x90, 0x90, 0x90]
        );

        s.disable("timer").unwrap();
        assert_eq!(
            mock.read_bytes(BASE + 0xC0, 3).unwrap(),
            vec![0x29, 0x43, 0x24]
        );
    }

    #[test]
    fn disabling_a_freeze_stops_it_being_written() {
        let (mock, s) = session();
        s.enable("health").unwrap();
        s.disable("health").unwrap();

        mock.poke(BASE + 0x40, &1i32.to_ne_bytes());
        s.tick();

        assert_eq!(
            mock.read_scalar(BASE + 0x40, ValueKind::I32).unwrap(),
            Scalar::I32(1)
        );
        assert!(!s.is_on("health"));
    }

    #[test]
    fn enabling_twice_is_harmless() {
        let (mock, s) = session();
        s.enable("timer").unwrap();
        s.enable("timer").unwrap();
        s.disable("timer").unwrap();
        assert_eq!(
            mock.read_bytes(BASE + 0xC0, 3).unwrap(),
            vec![0x29, 0x43, 0x24]
        );
    }

    #[test]
    fn a_cheat_that_cannot_resolve_is_refused_with_a_reason() {
        let (_, s) = session();
        match s.enable("orphan") {
            Err(Error::NotReady { reason, .. }) => assert!(reason.contains("not loaded")),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(!s.is_on("orphan"));
    }

    #[test]
    fn unknown_cheat_id_is_an_error() {
        let (_, s) = session();
        assert!(matches!(s.enable("nope"), Err(Error::NoSuchCheat(_))));
    }

    #[test]
    fn a_typed_number_is_what_gets_written() {
        let (mock, s) = session();
        s.choose("weight", "480").unwrap();
        s.enable("weight").unwrap();

        assert_eq!(
            mock.read_scalar(BASE + 0x100, ValueKind::I32).unwrap(),
            Scalar::I32(480)
        );
    }

    #[test]
    fn changing_the_number_while_it_is_on_takes_effect_there_and_then() {
        let (mock, s) = session();
        s.choose("weight", "480").unwrap();
        s.enable("weight").unwrap();
        s.choose("weight", "9000").unwrap();

        assert_eq!(
            mock.read_scalar(BASE + 0x100, ValueKind::I32).unwrap(),
            Scalar::I32(9000)
        );

        mock.poke(BASE + 0x100, &1i32.to_ne_bytes());
        s.tick();
        assert_eq!(
            mock.read_scalar(BASE + 0x100, ValueKind::I32).unwrap(),
            Scalar::I32(9000)
        );
    }

    #[test]
    fn a_value_with_nothing_typed_falls_back_to_whatever_the_game_holds() {
        let (mock, s) = session();
        mock.poke(BASE + 0x100, &37i32.to_ne_bytes());
        assert_eq!(s.value_for("weight"), Some(Scalar::I32(37)));
    }

    #[test]
    fn lock_off_writes_once_and_lets_go() {
        let (mock, s) = session();
        s.choose("speed", "2.5").unwrap();
        s.enable("speed").unwrap();
        assert_eq!(
            mock.read_scalar(BASE + 0x140, ValueKind::F32).unwrap(),
            Scalar::F32(2.5)
        );

        mock.poke(BASE + 0x140, &1.0f32.to_ne_bytes());
        s.tick();
        assert_eq!(
            mock.read_scalar(BASE + 0x140, ValueKind::F32).unwrap(),
            Scalar::F32(1.0)
        );
    }

    #[test]
    fn rubbish_in_the_box_is_refused_before_anything_is_written() {
        let (mock, s) = session();
        assert!(s.choose("weight", "lots").is_err());
        assert_eq!(
            mock.read_scalar(BASE + 0x100, ValueKind::I32).unwrap(),
            Scalar::I32(0)
        );
    }

    #[test]
    fn a_session_nobody_touched_does_not_count_as_used() {
        let (_, s) = session();
        assert!(!s.used());
    }

    #[test]
    fn arming_anything_counts_even_if_it_never_engages() {
        let (_, s) = session();
        s.arm("orphan").unwrap();
        assert!(s.used(), "they tried, and that it failed is the answer");
    }

    #[test]
    fn what_was_armed_last_launch_counts_too() {
        let (_, s) = session();
        s.arm_all(&["health".to_string()]);
        assert!(s.used());
    }

    #[test]
    fn disarming_does_not_undo_having_used_it() {
        let (_, s) = session();
        s.arm("health").unwrap();
        s.disarm("health").unwrap();
        assert!(s.used());
    }

    #[test]
    fn survey_reports_every_cheat() {
        let (_, s) = session();
        let states = s.survey();
        assert_eq!(states.len(), 6);

        let broken = states.iter().find(|(id, _)| id == "orphan").unwrap();
        assert!(matches!(broken.1, State::Broken { .. }));

        let ready = states.iter().find(|(id, _)| id == "health").unwrap();
        assert!(ready.1.is_ready());
    }

    #[test]
    fn disable_all_reverts_patches() {
        let (mock, s) = session();
        s.enable("health").unwrap();
        s.enable("timer").unwrap();
        assert_eq!(s.active_ids(), vec!["health", "timer"]);

        s.disable_all();

        assert!(s.active_ids().is_empty());
        assert_eq!(
            mock.read_bytes(BASE + 0xC0, 3).unwrap(),
            vec![0x29, 0x43, 0x24]
        );
    }

    #[test]
    fn the_worker_holds_a_value_without_manual_ticks() {
        let (mock, mut s) = session();
        s.enable("health").unwrap();
        s.start();

        mock.poke(BASE + 0x40, &0i32.to_ne_bytes());
        std::thread::sleep(TICK * 4);
        s.stop();

        assert_eq!(
            mock.read_scalar(BASE + 0x40, ValueKind::I32).unwrap(),
            Scalar::I32(999)
        );
    }
}
