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

    /* the bytes that were there before enabling are kept, so a [DISABLE] that
    will not build is not the end of it. plenty of them paste the aob back
    with the wildcards still in, which cannot be written, and the hook would
    otherwise stay in place until the game closed. */
    pub fn disable(&self, script: &Script, engaged: &Engaged) -> Result<()> {
        let writes = match self.plan(&script.disable, &engaged.symbols, false) {
            Ok(plan) => plan.writes,
            Err(e) if engaged.restores.is_empty() => return Err(e),
            Err(e) => {
                tracing::warn!("putting the original bytes back instead: {e}");
                engaged
                    .restores
                    .iter()
                    .map(|r| (r.addr, r.original.clone()))
                    .collect()
            }
        };

        for (addr, bytes) in &writes {
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
                let origin = self.locate(&section.anchor, &symbols)?;

                let body = expand_readmem(&section.body, |where_, size| {
                    let at = self.locate(where_, &symbols).ok()?;
                    self.target.read_bytes(at as usize, size).ok()
                });
                let built = self.build(&body, origin, &mut symbols)?;
                for (name, addr) in built.labels {
                    symbols.insert(name, addr);
                }
                if pass == 1 {
                    let mut bytes = built.bytes;
                    for hole in &built.holes {
                        if let Ok(now) = self.target.read_bytes(origin as usize + hole, 1) {
                            bytes[*hole] = now[0];
                        }
                    }
                    writes.push((origin as usize, bytes));
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

    /* `mov eax,[game.exe+1A2B3C]` is written straight into a line rather than
    scanned for first, so the assembler asks for a symbol nobody declared.
    every name it complains about gets one go at being a module before the
    error is passed on. */
    fn build(
        &self,
        body: &str,
        origin: u64,
        symbols: &mut HashMap<String, u64>,
    ) -> Result<freeplay_asm::Assembled> {
        let mut tried = 0;
        loop {
            let error = match assemble(body, origin, self.bits, symbols) {
                Ok(built) => return Ok(built),
                Err(e) => e,
            };
            let freeplay_asm::AsmError::UndefinedSymbol(name) = deepest(&error) else {
                return Err(error.into());
            };
            let (base, _) = script::split_offset(name);
            tried += 1;
            if tried > 32 || symbols.contains_key(base) {
                return Err(error.into());
            }
            // a pointer chain is followed here rather than encoded, since one
            // instruction cannot do two reads
            let found = match base.starts_with('[') {
                true => self.locate(base, symbols).ok(),
                false => self.target.module(base).ok().map(|m| m.base as u64),
            };
            match found {
                Some(addr) => symbols.insert(base.to_string(), addr),
                None => return Err(error.into()),
            };
        }
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

    // where a section starts writing. a bare label, a label plus an offset, or
    // a module plus an offset, because cheat engine uses all three
    fn locate(&self, anchor: &str, symbols: &HashMap<String, u64>) -> Result<u64> {
        if let Some(value) = symbols.get(anchor) {
            return Ok(*value);
        }

        let (base, offset) = script::split_offset(anchor);

        // `[gun_addy]:` writes wherever the pointer at gun_addy is pointing
        if let Some(inner) = base.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
            let at = self.locate(inner, symbols)?;
            let width = self.bits.pointer();
            let bytes = self.target.read_bytes(at as usize, width)?;
            let mut value = 0u64;
            for (n, byte) in bytes.iter().enumerate() {
                value |= (*byte as u64) << (8 * n);
            }
            let shift = offset.map_or(Ok(0), freeplay_asm::operand::number)?;
            return Ok(value.wrapping_add_signed(shift));
        }

        let start = match symbols.get(base) {
            Some(value) => *value,
            None => match self.target.module(base) {
                Ok(module) => module.base as u64,
                // `AmmoPatch+2+4:`, so peel one offset and go round again
                Err(_) if offset.is_some() && base != anchor => self.locate(base, symbols)?,
                Err(_) => return Err(AaError::UndefinedSymbol(anchor.to_string())),
            },
        };

        let shift = match offset {
            Some(text) => freeplay_asm::operand::number(text)
                .map_err(|_| AaError::UndefinedSymbol(anchor.to_string()))?,
            None => 0,
        };
        Ok(start.wrapping_add_signed(shift))
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

fn deepest(e: &freeplay_asm::AsmError) -> &freeplay_asm::AsmError {
    match e {
        freeplay_asm::AsmError::At { source, .. } => deepest(source),
        other => other,
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

/* `readmem(where,n)` puts the bytes that are at an address right now into the
script. tables use it in [DISABLE] to put back whatever they overwrote,
rather than spelling out the original instructions.

the assembler has no process to read from, so it happens here where there
is one, and what it produces is an ordinary `db`. thirteen thousand of the
corpus's complaints were this one directive.

an address that cannot be read leaves nops behind rather than failing the
whole script: a disable that restores nothing is bad, a disable that will
not assemble at all leaves the hook in place for ever. */
pub fn expand_readmem(body: &str, mut read: impl FnMut(&str, usize) -> Option<Vec<u8>>) -> String {
    if !body.to_ascii_lowercase().contains("readmem") {
        return body.to_string();
    }

    let mut out = String::with_capacity(body.len());
    for line in body.lines() {
        match one_readmem(line) {
            Some((before, where_, size, after)) => {
                let bytes = read(where_, size).unwrap_or_else(|| vec![0x90; size]);
                out.push_str(before);
                out.push_str("db ");
                for (n, byte) in bytes.iter().enumerate() {
                    if n > 0 {
                        out.push(' ');
                    }
                    out.push_str(&format!("{byte:02X}"));
                }
                out.push_str(after);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

fn one_readmem(line: &str) -> Option<(&str, &str, usize, &str)> {
    let lower = line.to_ascii_lowercase();
    let at = lower.find("readmem")?;
    let open = line[at..].find('(')? + at;
    let close = line[open..].find(')')? + open;

    let (where_, size) = line[open + 1..close].rsplit_once(',')?;
    let size = freeplay_asm::operand::number(size.trim()).ok()?;
    if !(1..=4096).contains(&size) {
        return None;
    }
    Some((
        &line[..at],
        where_.trim(),
        size as usize,
        &line[close + 1..],
    ))
}
