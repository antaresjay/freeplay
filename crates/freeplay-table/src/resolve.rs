use freeplay_core::error::Error as CoreError;
use freeplay_core::pattern::Pattern;
use freeplay_core::pointer::PointerPath;
use freeplay_core::scanner::{self, Scope as CoreScope};
use freeplay_core::target::Target;

use crate::schema::{Locator, Scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Ready { addr: usize },
    Unavailable { reason: String },
    Broken { reason: String },
}

impl State {
    pub fn addr(&self) -> Option<usize> {
        match self {
            State::Ready { addr } => Some(*addr),
            _ => None,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, State::Ready { .. })
    }
}

impl From<Scope> for CoreScope {
    fn from(scope: Scope) -> Self {
        match scope {
            Scope::Code => CoreScope::Code,
            Scope::Data => CoreScope::Data,
            Scope::All => CoreScope::Everything,
        }
    }
}

pub type Symbols = std::collections::HashMap<String, u64>;

pub fn evaluate(target: &dyn Target, locator: &Locator) -> State {
    evaluate_with(target, locator, &Symbols::new())
}

pub fn evaluate_with(target: &dyn Target, locator: &Locator, symbols: &Symbols) -> State {
    match locator {
        Locator::Symbol { symbol, hops } => {
            let Some(base) = symbols.get(symbol) else {
                return State::Unavailable {
                    reason: format!("{symbol} is not set until its script is switched on"),
                };
            };
            if *base == 0 {
                return State::Unavailable {
                    reason: format!("{symbol} has not been filled in yet"),
                };
            }
            follow(target, *base as usize, hops)
        }
        Locator::Static {
            module,
            offset,
            hops,
        } => {
            let base = match target.module(module) {
                Ok(m) => m.base,
                Err(_) => {
                    return State::Broken {
                        reason: format!("{module} is not loaded"),
                    };
                }
            };
            follow(target, base + offset, hops)
        }

        Locator::Pattern {
            pattern,
            scope,
            module,
            offset,
            rip,
            hops,
        } => {
            let compiled = match Pattern::parse(pattern) {
                Ok(p) => p,
                Err(e) => {
                    return State::Broken {
                        reason: e.to_string(),
                    }
                }
            };

            let search_scope = match module {
                Some(name) => CoreScope::Module(name.clone()),
                None => (*scope).into(),
            };

            let found = match scanner::find_one(target, &compiled, search_scope) {
                Ok(addr) => addr,
                Err(CoreError::NotFound) => {
                    return State::Broken {
                        reason: "signature not found, the game has probably been patched".into(),
                    };
                }
                Err(CoreError::Ambiguous { found }) => {
                    return State::Broken {
                        reason: format!(
                            "signature matches {found} places, it is not specific enough"
                        ),
                    };
                }
                Err(e) => {
                    return State::Broken {
                        reason: e.to_string(),
                    }
                }
            };

            let mut addr = found.wrapping_add_signed(*offset as isize);

            if let Some(rip) = rip {
                let raw = match target.read_bytes(found + rip.displacement_at, 4) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        return State::Broken {
                            reason: e.to_string(),
                        }
                    }
                };
                let displacement = i32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]) as isize;
                addr = (found + rip.instruction_length).wrapping_add_signed(displacement);
            }

            follow(target, addr, hops)
        }
    }
}

fn follow(target: &dyn Target, start: usize, hops: &[crate::schema::Hop]) -> State {
    if hops.is_empty() {
        return State::Ready { addr: start };
    }

    let path = PointerPath::absolute(start, hops.iter().map(|h| h.0).collect());
    match path.resolve(target) {
        Ok(addr) => State::Ready { addr },
        Err(CoreError::BrokenChain { hop, .. }) => State::Unavailable {
            reason: format!(
                "pointer is empty at step {}, load into the game first",
                hop + 1
            ),
        },
        Err(e) => State::Unavailable {
            reason: e.to_string(),
        },
    }
}

pub fn parse_bytes(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        if token.len() % 2 != 0 {
            return Err(format!("odd length token {token:?}"));
        }
        for pair in token.as_bytes().chunks(2) {
            let pair = std::str::from_utf8(pair).map_err(|_| "not ascii".to_string())?;
            out.push(u8::from_str_radix(pair, 16).map_err(|_| format!("bad byte {pair:?}"))?);
        }
    }
    if out.is_empty() {
        return Err("no bytes given".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use freeplay_core::mock::MockTarget;

    use crate::schema::{Hop, Rip};

    const BASE: usize = 0x5000_0000;

    fn target() -> MockTarget {
        MockTarget::zeroed(BASE, 0x2000).with_module("game.exe", BASE, 0x2000)
    }

    #[test]
    fn static_locator_with_no_hops_is_the_address_itself() {
        let t = target();
        let locator = Locator::Static {
            module: "game.exe".into(),
            offset: 0x40,
            hops: vec![],
        };
        assert_eq!(evaluate(&t, &locator), State::Ready { addr: BASE + 0x40 });
    }

    #[test]
    fn static_locator_follows_hops() {
        let t = target();
        t.poke_usize(BASE + 0x40, BASE + 0x200);
        t.poke_usize(BASE + 0x210, BASE + 0x800);

        let locator = Locator::Static {
            module: "game.exe".into(),
            offset: 0x40,
            hops: vec![Hop(0x10), Hop(0x4)],
        };
        assert_eq!(evaluate(&t, &locator), State::Ready { addr: BASE + 0x804 });
    }

    #[test]
    fn missing_module_is_broken_not_unavailable() {
        let t = target();
        let locator = Locator::Static {
            module: "other.exe".into(),
            offset: 0,
            hops: vec![],
        };
        assert!(matches!(evaluate(&t, &locator), State::Broken { .. }));
    }

    #[test]
    fn empty_pointer_is_unavailable_not_broken() {
        let t = target();
        let locator = Locator::Static {
            module: "game.exe".into(),
            offset: 0x40,
            hops: vec![Hop(0x10)],
        };
        match evaluate(&t, &locator) {
            State::Unavailable { reason } => assert!(reason.contains("load into the game")),
            other => panic!("expected unavailable, got {other:?}"),
        }
    }

    #[test]
    fn pattern_locator_finds_the_match() {
        let t = MockTarget::zeroed(BASE, 0x2000).with_module("game.exe", BASE, 0x2000);
        t.poke(BASE + 0x300, &[0x48, 0x8B, 0x05, 0x11, 0x22, 0x33, 0x44]);

        let locator = Locator::Pattern {
            pattern: "48 8B 05 ?? ?? ?? ??".into(),
            scope: Scope::Data,
            module: None,
            offset: 0,
            rip: None,
            hops: vec![],
        };
        assert_eq!(evaluate(&t, &locator), State::Ready { addr: BASE + 0x300 });
    }

    #[test]
    fn missing_signature_says_the_game_was_patched() {
        let t = target();
        let locator = Locator::Pattern {
            pattern: "DE AD BE EF CA FE".into(),
            scope: Scope::Data,
            module: None,
            offset: 0,
            rip: None,
            hops: vec![],
        };
        match evaluate(&t, &locator) {
            State::Broken { reason } => assert!(reason.contains("patched")),
            other => panic!("expected broken, got {other:?}"),
        }
    }

    #[test]
    fn offset_shifts_the_match() {
        let t = MockTarget::zeroed(BASE, 0x2000).with_module("game.exe", BASE, 0x2000);
        t.poke(BASE + 0x300, &[0x48, 0x8B, 0x05, 0x11]);

        let locator = Locator::Pattern {
            pattern: "48 8B 05".into(),
            scope: Scope::Data,
            module: None,
            offset: 3,
            rip: None,
            hops: vec![],
        };
        assert_eq!(evaluate(&t, &locator), State::Ready { addr: BASE + 0x303 });
    }

    #[test]
    fn rip_relative_operand_is_followed() {
        let t = MockTarget::zeroed(BASE, 0x2000).with_module("game.exe", BASE, 0x2000);
        let mut code = vec![0x48, 0x8B, 0x05];
        code.extend_from_slice(&0x100i32.to_ne_bytes());
        t.poke(BASE + 0x300, &code);

        let locator = Locator::Pattern {
            pattern: "48 8B 05 ?? ?? ?? ??".into(),
            scope: Scope::Data,
            module: None,
            offset: 0,
            rip: Some(Rip {
                displacement_at: 3,
                instruction_length: 7,
            }),
            hops: vec![],
        };
        assert_eq!(
            evaluate(&t, &locator),
            State::Ready {
                addr: BASE + 0x300 + 7 + 0x100
            }
        );
    }

    #[test]
    fn parses_replacement_bytes() {
        assert_eq!(parse_bytes("90 90 90").unwrap(), vec![0x90, 0x90, 0x90]);
        assert_eq!(parse_bytes("31C0C3").unwrap(), vec![0x31, 0xC0, 0xC3]);
        assert!(parse_bytes("").is_err());
        assert!(parse_bytes("9").is_err());
        assert!(parse_bytes("ZZ").is_err());
    }
}
