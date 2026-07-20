use freeplay_core::target::Module;

use crate::script::{Directive, Script};

// a downloaded table is somebody else's code running inside your game. these
// are the things a cheat never needs to do, so refusing them costs nothing and
// closes the obvious ways to turn this into a malware channel
pub const MAX_ALLOC: usize = 1 << 20;
pub const MAX_ALLOCS: usize = 32;
pub const MAX_HOOKS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub what: String,
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.what, self.why)
    }
}

// directives that load or run something of their own choosing. we already skip
// these when a table is local, but a table off the network does not get to ask
pub(crate) const BANNED: &[&str] = &[
    "loadlibrary",
    "createthread",
    "createthreadandwait",
    "luacall",
    "luacode",
    "shellexecute",
    "winexec",
];

pub fn check(script: &Script) -> Vec<Refusal> {
    let mut refusals = Vec::new();
    let mut allocs = 0usize;

    for half in [&script.enable, &script.disable] {
        for directive in &half.directives {
            match directive {
                Directive::Alloc { symbol, size, .. } => {
                    allocs += 1;
                    if *size > MAX_ALLOC {
                        refusals.push(Refusal {
                            what: symbol.clone(),
                            why: format!("wants {size} bytes, more than a cheat has any use for"),
                        });
                    }
                }
                Directive::Ignored(text) => {
                    let head = text
                        .split('(')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_lowercase();
                    if BANNED.contains(&head.as_str()) {
                        refusals.push(Refusal {
                            what: head,
                            why: "loads or runs code of its own choosing".into(),
                        });
                    }
                }
                _ => {}
            }
        }

        if half.sections.len() > MAX_HOOKS {
            refusals.push(Refusal {
                what: format!("{} sections", half.sections.len()),
                why: "far more hooks than a cheat table needs".into(),
            });
        }
    }

    if allocs > MAX_ALLOCS {
        refusals.push(Refusal {
            what: format!("{allocs} allocations"),
            why: "more code caves than a cheat table needs".into(),
        });
    }

    refusals
}

// every address a script writes to has to land inside a module the game itself
// loaded, or in a cave we handed it. writing into a system dll is not a cheat
pub fn writes_stay_inside(
    writes: &[(usize, usize)],
    modules: &[Module],
    caves: &[(usize, usize)],
) -> Vec<Refusal> {
    let mut refusals = Vec::new();

    for (addr, len) in writes {
        let end = addr.saturating_add(*len);
        let in_cave = caves
            .iter()
            .any(|(base, size)| *addr >= *base && end <= base.saturating_add(*size));
        if in_cave {
            continue;
        }

        let home = modules
            .iter()
            .find(|m| *addr >= m.base && end <= m.end())
            .map(|m| m.name.to_ascii_lowercase());

        match home {
            Some(name) if is_system(&name) => refusals.push(Refusal {
                what: format!("{addr:#x}"),
                why: format!("writes into {name}, which belongs to windows and not the game"),
            }),
            Some(_) => {}
            None => refusals.push(Refusal {
                what: format!("{addr:#x}"),
                why: "writes somewhere no module or cave covers".into(),
            }),
        }
    }

    refusals
}

fn is_system(module: &str) -> bool {
    const SYSTEM: &[&str] = &[
        "ntdll.dll",
        "kernel32.dll",
        "kernelbase.dll",
        "user32.dll",
        "advapi32.dll",
        "ws2_32.dll",
        "wininet.dll",
        "winhttp.dll",
        "urlmon.dll",
        "shell32.dll",
        "ole32.dll",
        "crypt32.dll",
        "bcrypt.dll",
        "secur32.dll",
        "amsi.dll",
    ];
    SYSTEM.contains(&module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn script(body: &str) -> Script {
        parse(body).unwrap()
    }

    #[test]
    fn an_ordinary_table_is_fine() {
        let s = script(
            "[ENABLE]\nalloc(cave,100,here)\nlabel(slot)\ncave:\n  nop\n[DISABLE]\ndealloc(cave)\n",
        );
        assert!(check(&s).is_empty());
    }

    #[test]
    fn loading_a_dll_is_refused() {
        let s =
            script("[ENABLE]\nloadlibrary(evil.dll)\nalloc(cave,100)\ncave:\n  nop\n[DISABLE]\n");
        let out = check(&s);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].what, "loadlibrary");
    }

    #[test]
    fn spawning_a_thread_is_refused() {
        let s = script("[ENABLE]\ncreatethread(cave)\nalloc(cave,100)\ncave:\n  nop\n[DISABLE]\n");
        assert_eq!(check(&s)[0].what, "createthread");
    }

    #[test]
    fn a_silly_allocation_is_refused() {
        let s = script("[ENABLE]\nalloc(cave,10000000)\ncave:\n  nop\n[DISABLE]\n");
        let out = check(&s);
        assert_eq!(out.len(), 1);
        assert!(out[0].why.contains("more than a cheat"));
    }

    #[test]
    fn a_write_into_the_game_is_allowed() {
        let modules = vec![Module {
            name: "game.exe".into(),
            base: 0x40_0000,
            size: 0x10_0000,
        }];
        assert!(writes_stay_inside(&[(0x41_0000, 10)], &modules, &[]).is_empty());
    }

    #[test]
    fn a_write_into_ntdll_is_refused() {
        let modules = vec![
            Module {
                name: "game.exe".into(),
                base: 0x40_0000,
                size: 0x10_0000,
            },
            Module {
                name: "ntdll.dll".into(),
                base: 0x7000_0000,
                size: 0x10_0000,
            },
        ];
        let out = writes_stay_inside(&[(0x7000_1000, 10)], &modules, &[]);
        assert_eq!(out.len(), 1);
        assert!(out[0].why.contains("ntdll.dll"));
    }

    #[test]
    fn a_write_into_our_own_cave_is_allowed() {
        let modules = vec![Module {
            name: "game.exe".into(),
            base: 0x40_0000,
            size: 0x1000,
        }];
        let caves = vec![(0x900_0000, 0x100)];
        assert!(writes_stay_inside(&[(0x900_0010, 16)], &modules, &caves).is_empty());
    }

    #[test]
    fn a_write_into_nowhere_is_refused() {
        let modules = vec![Module {
            name: "game.exe".into(),
            base: 0x40_0000,
            size: 0x1000,
        }];
        let out = writes_stay_inside(&[(0x1234_5678, 4)], &modules, &[]);
        assert_eq!(out.len(), 1);
        assert!(out[0].why.contains("no module or cave"));
    }

    #[test]
    fn a_write_running_off_the_end_of_a_module_is_refused() {
        let modules = vec![Module {
            name: "game.exe".into(),
            base: 0x40_0000,
            size: 0x1000,
        }];
        assert_eq!(
            writes_stay_inside(&[(0x40_0FF8, 32)], &modules, &[]).len(),
            1
        );
    }
}
