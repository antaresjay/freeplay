pub mod error;
pub mod safety;
pub mod script;

use std::collections::HashMap;

use freeplay_asm::{assemble, Bits};
use freeplay_core::pattern::Pattern;
use freeplay_core::scanner::{self, Scope};
use freeplay_core::target::Arch;
use freeplay_core::Target;

pub use error::{AaError, Result};
pub use script::{parse, Directive, Script, Section};

#[derive(Debug, Clone)]
pub struct Restore {
    pub addr: usize,
    pub original: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Engaged {
    pub symbols: HashMap<String, u64>,
    pub allocations: Vec<usize>,
    pub restores: Vec<Restore>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub writes: Vec<(usize, Vec<u8>)>,
    pub symbols: HashMap<String, u64>,
    pub allocations: Vec<usize>,
    pub registered: Vec<String>,
}

pub struct Runner<'a> {
    target: &'a dyn Target,
    bits: Bits,
    // a table off the network gets the rules applied. one you wrote or dropped
    // in yourself does not, it is your machine
    guarded: bool,
}

impl<'a> Runner<'a> {
    pub fn new(target: &'a dyn Target) -> Self {
        let bits = match target.arch() {
            Arch::X86 => Bits::X86,
            Arch::X64 => Bits::X64,
        };
        Self {
            target,
            bits,
            guarded: false,
        }
    }

    pub fn guarded(mut self) -> Self {
        self.guarded = true;
        self
    }

    pub fn enable(&self, script: &Script, known: &HashMap<String, u64>) -> Result<Engaged> {
        if self.guarded {
            let refusals = safety::check(script);
            if !refusals.is_empty() {
                return Err(AaError::Refused { refusals });
            }
        }

        let plan = self.plan(&script.enable, known, true)?;

        if self.guarded {
            let modules = self.target.modules()?;
            let caves: Vec<(usize, usize)> = plan
                .allocations
                .iter()
                .map(|addr| (*addr, safety::MAX_ALLOC))
                .collect();
            let writes: Vec<(usize, usize)> = plan
                .writes
                .iter()
                .map(|(addr, bytes)| (*addr, bytes.len()))
                .collect();

            let refusals = safety::writes_stay_inside(&writes, &modules, &caves);
            if !refusals.is_empty() {
                for addr in &plan.allocations {
                    let _ = self.target.release(*addr);
                }
                return Err(AaError::Refused { refusals });
            }
        }

        self.apply(plan)
    }

    pub fn disable(&self, script: &Script, engaged: &Engaged) -> Result<()> {
        let plan = self.plan(&script.disable, &engaged.symbols, false)?;

        for (addr, bytes) in &plan.writes {
            self.write(*addr, bytes)?;
        }
        for addr in &engaged.allocations {
            let _ = self.target.release(*addr);
        }
        Ok(())
    }

    fn plan(
        &self,
        half: &script::Half,
        known: &HashMap<String, u64>,
        allocate: bool,
    ) -> Result<Plan> {
        let mut symbols = known.clone();
        let mut allocations = Vec::new();
        let mut registered = Vec::new();
        let mut declared = Vec::new();
        let mut asserts = Vec::new();

        for directive in &half.directives {
            match directive {
                Directive::AobScanModule {
                    symbol,
                    module,
                    pattern,
                } => {
                    let addr = self.scan(pattern, Some(module), symbol)?;
                    symbols.insert(symbol.clone(), addr as u64);
                }
                Directive::AobScan { symbol, pattern } => {
                    let addr = self.scan(pattern, None, symbol)?;
                    symbols.insert(symbol.clone(), addr as u64);
                }
                Directive::Alloc { symbol, size, near } => {
                    if !allocate {
                        continue;
                    }
                    let anchor = near
                        .as_ref()
                        .and_then(|n| symbols.get(n))
                        .map(|a| *a as usize);
                    let addr = self.target.allocate(*size, anchor)?;
                    allocations.push(addr);
                    symbols.insert(symbol.clone(), addr as u64);
                }
                Directive::Label(name) => declared.push(name.clone()),
                Directive::RegisterSymbol(name) => registered.push(name.clone()),
                Directive::Define { name, value } => {
                    if let Ok(number) = freeplay_asm::operand::number(value) {
                        symbols.insert(name.clone(), number as u64);
                    }
                }
                Directive::Assert { symbol, bytes } => {
                    asserts.push((symbol.clone(), bytes.clone()))
                }
                Directive::UnregisterSymbol(_) | Directive::Dealloc(_) | Directive::Ignored(_) => {}
            }
        }

        for (symbol, bytes) in &asserts {
            self.check_assert(symbol, bytes, &symbols)?;
        }

        for name in &declared {
            symbols.entry(name.clone()).or_insert(0);
        }

        let mut writes = Vec::new();
        for pass in 0..2 {
            writes.clear();
            for section in &half.sections {
                let origin = *symbols
                    .get(&section.anchor)
                    .ok_or_else(|| AaError::UndefinedSymbol(section.anchor.clone()))?;

                let built = assemble(&section.body, origin, self.bits, &symbols)?;
                for (name, addr) in built.labels {
                    symbols.insert(name, addr);
                }
                if pass == 1 {
                    writes.push((origin as usize, built.bytes));
                }
            }
        }

        Ok(Plan {
            writes,
            symbols,
            allocations,
            registered,
        })
    }

    fn apply(&self, plan: Plan) -> Result<Engaged> {
        let mut restores = Vec::new();

        for (addr, bytes) in &plan.writes {
            let original = self
                .target
                .read_bytes(*addr, bytes.len())
                .unwrap_or_default();
            if original.len() == bytes.len() {
                restores.push(Restore {
                    addr: *addr,
                    original,
                });
            }
        }

        for (addr, bytes) in &plan.writes {
            self.write(*addr, bytes)?;
        }

        Ok(Engaged {
            symbols: plan.symbols,
            allocations: plan.allocations,
            restores,
        })
    }

    fn write(&self, addr: usize, bytes: &[u8]) -> Result<()> {
        let previous = self.target.make_writable(addr, bytes.len())?;
        let outcome = self.target.write_bytes(addr, bytes);
        let _ = self.target.restore_protection(addr, bytes.len(), previous);
        outcome?;

        let back = self.target.read_bytes(addr, bytes.len())?;
        if back != bytes {
            return Err(AaError::Target(freeplay_core::Error::WriteFailed {
                addr,
                len: bytes.len(),
                source: std::io::Error::other("the write did not stick"),
            }));
        }
        Ok(())
    }

    fn scan(&self, pattern: &str, module: Option<&str>, symbol: &str) -> Result<usize> {
        let compiled = Pattern::parse(pattern).map_err(|_| AaError::SignatureMissing {
            symbol: symbol.to_string(),
        })?;

        let scope = match module {
            Some(name) => Scope::Module(name.to_string()),
            None => Scope::Code,
        };

        let hits = scanner::find_all(self.target, &compiled, scope)?;
        match hits.first() {
            Some(addr) => {
                if hits.len() > 1 {
                    tracing::warn!(
                        "{symbol} matched {} places, taking the first at {:#x}",
                        hits.len(),
                        addr
                    );
                }
                Ok(*addr)
            }
            None => Err(AaError::SignatureMissing {
                symbol: symbol.to_string(),
            }),
        }
    }

    fn check_assert(
        &self,
        symbol: &str,
        bytes: &str,
        symbols: &HashMap<String, u64>,
    ) -> Result<()> {
        let Some(addr) = symbols.get(symbol) else {
            return Ok(());
        };
        let Ok(pattern) = Pattern::parse(bytes) else {
            return Ok(());
        };
        let seen = self.target.read_bytes(*addr as usize, pattern.len())?;
        if pattern.matches_at(&seen, 0) {
            Ok(())
        } else {
            Err(AaError::AssertFailed {
                symbol: symbol.to_string(),
            })
        }
    }
}

pub fn symbols_defined(script: &Script) -> Vec<String> {
    let mut out = Vec::new();
    for directive in &script.enable.directives {
        match directive {
            Directive::RegisterSymbol(name) => out.push(name.clone()),
            Directive::Alloc { symbol, .. } => out.push(symbol.clone()),
            Directive::AobScanModule { symbol, .. } | Directive::AobScan { symbol, .. } => {
                out.push(symbol.clone())
            }
            Directive::Label(name) => out.push(name.clone()),
            _ => {}
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}
