use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use freeplay_aa::{Runner, Script};
use freeplay_core::patch::Patch;
use freeplay_core::target::Target;
use freeplay_core::value::Scalar;
use freeplay_table::resolve::{self, State, Symbols};
use freeplay_table::schema::{Action, Cheat};
use freeplay_table::Table;

pub const TICK: Duration = Duration::from_millis(30);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no cheat called {0}")]
    NoSuchCheat(String),

    #[error("{name} is not available: {reason}")]
    NotReady { name: String, reason: String },

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

    fn engage(&self, cheat: &Cheat, addr: usize) -> Result<Engaged, Error> {
        match &cheat.action {
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
    fn survey_reports_every_cheat() {
        let (_, s) = session();
        let states = s.survey();
        assert_eq!(states.len(), 4);

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
