use crate::error::{AaError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    AobScanModule {
        symbol: String,
        module: String,
        pattern: String,
    },
    AobScan {
        symbol: String,
        pattern: String,
    },
    Alloc {
        symbol: String,
        size: usize,
        near: Option<String>,
    },
    Label(String),
    RegisterSymbol(String),
    UnregisterSymbol(String),
    Dealloc(String),
    Define {
        name: String,
        value: String,
    },
    Assert {
        symbol: String,
        bytes: String,
    },
    Ignored(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub anchor: String,
    pub body: String,
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Half {
    pub directives: Vec<Directive>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    pub enable: Half,
    pub disable: Half,
}

pub fn parse(source: &str) -> Result<Script> {
    // cheat engine lets a script switch language partway through. lua and c
    // both get the whole machine rather than the game's memory, so a table off
    // the network does not get to use them, and saying which it was beats
    // failing on the first line of it that does not look like assembly
    if let Some(other) = other_language(source) {
        return Err(AaError::NotAssembly(other));
    }

    let mut enable = Vec::new();
    let mut disable = Vec::new();
    let mut active = None;

    for (index, raw) in source.lines().enumerate() {
        match raw.trim().to_ascii_uppercase().as_str() {
            "[ENABLE]" => {
                active = Some(true);
                continue;
            }
            "[DISABLE]" => {
                active = Some(false);
                continue;
            }
            _ => {}
        }

        match active {
            Some(true) => enable.push((index + 1, raw.to_string())),
            Some(false) => disable.push((index + 1, raw.to_string())),
            None => {}
        }
    }

    Ok(Script {
        enable: read_half(&enable)?,
        disable: read_half(&disable)?,
    })
}

fn read_half(lines: &[(usize, String)]) -> Result<Half> {
    let mut half = Half::default();
    let mut depth = 0usize;
    let mut section: Option<Section> = None;
    let inner = declared_labels(lines);

    for (line, raw) in lines {
        let mut text = raw.clone();

        loop {
            if depth > 0 {
                match text.find('}') {
                    Some(at) => {
                        depth -= 1;
                        text = text[at + 1..].to_string();
                    }
                    None => {
                        text.clear();
                        break;
                    }
                }
            } else {
                match text.find('{') {
                    Some(at) => {
                        depth += 1;
                        text.truncate(at);
                        break;
                    }
                    None => break,
                }
            }
        }

        let mut body = text.as_str();
        if let Some(at) = body.find("//") {
            body = &body[..at];
        }
        let trimmed = body.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(directive) = read_directive(trimmed)? {
            if let Some(open) = section.take() {
                half.sections.push(open);
            }
            half.directives.push(directive);
            continue;
        }

        if let Some(name) = anchor_label(trimmed) {
            if !inner.contains(&name) {
                if let Some(open) = section.take() {
                    half.sections.push(open);
                }
                section = Some(Section {
                    anchor: name,
                    body: String::new(),
                    line: *line,
                });
                continue;
            }
        }

        match section.as_mut() {
            Some(open) => {
                open.body.push_str(trimmed);
                open.body.push('\n');
            }
            None => {
                return Err(AaError::Stray {
                    line: *line,
                    text: trimmed.to_string(),
                })
            }
        }
    }

    if let Some(open) = section.take() {
        half.sections.push(open);
    }
    Ok(half)
}

fn declared_labels(lines: &[(usize, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (_, raw) in lines {
        let mut text = raw.as_str();
        if let Some(at) = text.find("//") {
            text = &text[..at];
        }
        if let Ok(Some(Directive::Label(name))) = read_directive(text.trim()) {
            out.push(name);
        }
    }
    out
}

// "{$lua}" and friends, which turn the rest of the script into another language
fn other_language(source: &str) -> Option<&'static str> {
    let lowered = source.to_ascii_lowercase();
    for (marker, name) in [
        ("{$lua", "Lua"),
        ("{$luacode", "Lua"),
        ("{$ccode", "C"),
        ("{$c}", "C"),
    ] {
        if lowered.contains(marker) {
            return Some(name);
        }
    }
    None
}

fn anchor_label(text: &str) -> Option<String> {
    let at = text.find(':')?;
    let name = text[..at].trim();
    if !text[at + 1..].trim().is_empty() {
        return None;
    }

    // "god+07:" and "game.exe+1A2B3C:" are both somewhere to start writing, and
    // tables use them as freely as a plain label. anything cleverer than one
    // offset, like "aob+(DWORD)[aob+03]+07:", is left for a person to look at
    let (base, _) = split_offset(name);
    if base.is_empty()
        || !base
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        || !base
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return None;
    }
    Some(name.to_string())
}

// "god+07" into "god" and "+07". the tail has to be a number or there is no
// split, so a module called some-game.exe stays in one piece. a module with a
// space in its name comes quoted, and the quotes are not part of the name
pub fn split_offset(name: &str) -> (&str, Option<&str>) {
    let (head, tail) = match name.rfind(['+', '-']) {
        Some(at) if at > 0 && freeplay_asm::operand::number(&name[at..]).is_ok() => {
            (&name[..at], Some(&name[at..]))
        }
        _ => (name, None),
    };
    (head.trim().trim_matches('"'), tail)
}

fn read_directive(text: &str) -> Result<Option<Directive>> {
    let Some(open) = text.find('(') else {
        return Ok(None);
    };
    let name = text[..open].trim().to_ascii_lowercase();
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok(None);
    }
    let Some(close) = text.rfind(')') else {
        return Ok(None);
    };
    if close < open {
        return Ok(None);
    }
    let inner = &text[open + 1..close];
    let args: Vec<String> = split_args(inner);

    let arg = |n: usize| -> Result<String> {
        args.get(n)
            .map(|a| a.trim().to_string())
            .ok_or_else(|| AaError::BadDirective(text.to_string()))
    };

    let directive = match name.as_str() {
        "aobscanmodule" | "aobscanmoduleunique" => Directive::AobScanModule {
            symbol: arg(0)?,
            module: arg(1)?,
            pattern: args[2..].join(" "),
        },
        "aobscan" | "aobscanregion" => Directive::AobScan {
            symbol: arg(0)?,
            pattern: args[1..].join(" "),
        },
        "alloc" | "globalalloc" => Directive::Alloc {
            symbol: arg(0)?,
            size: freeplay_asm::operand::number(&arg(1)?)
                .unwrap_or(0x1000)
                .max(1) as usize,
            near: args.get(2).map(|a| a.trim().to_string()),
        },
        "label" => Directive::Label(arg(0)?),
        "registersymbol" => Directive::RegisterSymbol(arg(0)?),
        "unregistersymbol" => Directive::UnregisterSymbol(arg(0)?),
        "dealloc" => Directive::Dealloc(arg(0)?),
        "define" => Directive::Define {
            name: arg(0)?,
            value: args[1..].join(","),
        },
        "assert" => Directive::Assert {
            symbol: arg(0)?,
            bytes: args[1..].join(" "),
        },
        "createthread"
        | "loadlibrary"
        | "luacall"
        | "aobscanall"
        | "fullaccess"
        | "createthreadandwait"
        | "reassemble"
        | "unlockmemory" => Directive::Ignored(text.to_string()),
        _ => return Ok(None),
    };

    Ok(Some(directive))
}

fn split_args(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();

    for ch in inner.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                out.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITCHER: &str = r#"{
    Game    : witcher2.EXE
}
[ENABLE]
aobscanmodule(getWitcher,witcher2.EXE,8B 10 8B C8 FF 92 34 02 00 00 84)
registersymbol(getWitcher)
alloc(newgetWitcher,100,getWitcher)
label(codegetWitcher)
label(baseWitcher)
newgetWitcher:
  mov [baseWitcher],eax
codegetWitcher:
  mov edx,[eax]
baseWitcher:
  dd 0
getWitcher:
  jmp newgetWitcher
  nop 5

[DISABLE]
getWitcher:
  db 8B 10 8B C8 FF 92 34 02 00 00
unregistersymbol(getWitcher)
dealloc(newgetWitcher)
"#;

    #[test]
    fn splits_enable_from_disable() {
        let script = parse(WITCHER).unwrap();
        assert_eq!(script.enable.sections.len(), 2);
        assert_eq!(script.disable.sections.len(), 1);
    }

    #[test]
    fn reads_the_scan_and_the_allocation() {
        let script = parse(WITCHER).unwrap();
        assert!(script
            .enable
            .directives
            .contains(&Directive::AobScanModule {
                symbol: "getWitcher".into(),
                module: "witcher2.EXE".into(),
                pattern: "8B 10 8B C8 FF 92 34 02 00 00 84".into(),
            }));
        assert!(script.enable.directives.contains(&Directive::Alloc {
            symbol: "newgetWitcher".into(),
            size: 0x100,
            near: Some("getWitcher".into()),
        }));
    }

    #[test]
    fn the_header_comment_block_is_not_code() {
        let script = parse(WITCHER).unwrap();
        assert!(!script
            .enable
            .sections
            .iter()
            .any(|s| s.body.contains("witcher2.EXE")));
    }

    #[test]
    fn a_section_keeps_the_lines_under_its_anchor() {
        let script = parse(WITCHER).unwrap();
        let cave = &script.enable.sections[0];
        assert_eq!(cave.anchor, "newgetWitcher");
        assert!(cave.body.contains("mov [baseWitcher],eax"));
        assert!(cave.body.contains("codegetWitcher:"));
        assert!(cave.body.contains("dd 0"));

        let hook = &script.enable.sections[1];
        assert_eq!(hook.anchor, "getWitcher");
        assert!(hook.body.contains("jmp newgetWitcher"));
    }

    #[test]
    fn disable_puts_the_original_bytes_back() {
        let script = parse(WITCHER).unwrap();
        assert_eq!(script.disable.sections[0].anchor, "getWitcher");
        assert!(script.disable.sections[0].body.contains("db 8B 10"));
        assert!(script
            .disable
            .directives
            .contains(&Directive::Dealloc("newgetWitcher".into())));
    }

    #[test]
    fn the_trailing_disassembly_block_is_dropped() {
        let source = "[ENABLE]\nalloc(a,100)\na:\n  nop\n{\nwitcher2.EXE+1: 8B 10 - mov edx,[eax]\n}\n[DISABLE]\n";
        let script = parse(source).unwrap();
        assert_eq!(script.enable.sections[0].body.trim(), "nop");
    }

    // patching a few bytes into the middle of a match, rather than hooking the
    // top of it, is how a good third of real tables are written
    #[test]
    fn a_section_can_start_partway_into_a_symbol() {
        let source =
            "[ENABLE]\naobscanmodule(god,game.exe,0F B6 4B)\ngod+07:\n  xor esi,esi\n[DISABLE]\n";
        let script = parse(source).unwrap();
        assert_eq!(script.enable.sections[0].anchor, "god+07");
        assert_eq!(script.enable.sections[0].body.trim(), "xor esi,esi");
    }

    #[test]
    fn a_section_can_start_at_a_module() {
        let source = "[ENABLE]\ngame.exe+1A2B3C:\n  nop\n[DISABLE]\n";
        let script = parse(source).unwrap();
        assert_eq!(script.enable.sections[0].anchor, "game.exe+1A2B3C");
    }

    #[test]
    fn an_offset_that_is_not_a_number_is_not_a_place_to_write() {
        assert_eq!(anchor_label("cmdAob+(DWORD)[cmdAob+03]+07:"), None);
        assert_eq!(anchor_label("mov [rax],1"), None);
    }

    #[test]
    fn splitting_leaves_a_hyphenated_name_alone() {
        assert_eq!(split_offset("some-game.exe"), ("some-game.exe", None));
        assert_eq!(split_offset("god+07"), ("god", Some("+07")));
        assert_eq!(split_offset("god-4"), ("god", Some("-4")));
    }

    // a module with a space in it is written in quotes, and they are not part
    // of the name
    #[test]
    fn a_quoted_module_loses_its_quotes() {
        assert_eq!(
            split_offset("\"sekiro.exe\"+BAD636"),
            ("sekiro.exe", Some("+BAD636"))
        );
        let script = parse("[ENABLE]\n\"sekiro.exe\"+BAD636:\n  db 90\n[DISABLE]\n").unwrap();
        assert_eq!(script.enable.sections[0].anchor, "\"sekiro.exe\"+BAD636");
    }

    #[test]
    fn a_script_that_switches_to_lua_is_refused_by_name() {
        let source = "[ENABLE]\n{$lua}\nif syntaxcheck then return end\nprint('hi')\n[DISABLE]\n";
        let why = parse(source).unwrap_err().to_string();
        assert!(why.contains("Lua"), "{why}");
    }

    #[test]
    fn a_c_block_is_refused_the_same_way() {
        assert!(parse("[ENABLE]\n{$ccode}\nint x;\n[DISABLE]\n")
            .unwrap_err()
            .to_string()
            .contains('C'));
    }

    // the ordinary comment block still has to work
    #[test]
    fn a_plain_brace_block_is_still_just_a_comment() {
        let script = parse("[ENABLE]\nalloc(a,100)\na:\n  nop\n{ notes }\n[DISABLE]\n").unwrap();
        assert_eq!(script.enable.sections[0].body.trim(), "nop");
    }
}
