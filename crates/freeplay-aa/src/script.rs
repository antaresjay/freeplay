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
    let mut header = Vec::new();
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
            // the header block lives up here and is nobody's code. we drop it
            // rather than run it, but a table that hid a loadlibrary in there
            // is still a table nobody should be passing round
            None => {
                if let Some(what) = banned_call(raw) {
                    return Err(AaError::Refused {
                        refusals: vec![crate::safety::Refusal {
                            what: what.to_string(),
                            why: "sits above [ENABLE] where it would be easy to miss".into(),
                        }],
                    });
                }
                // a define up here still counts, and tables put them there
                header.push((index + 1, raw.to_string()));
            }
        }
    }

    /* `define(ToolDurability,"UDK.exe"+1EECCB)` stands for a piece of text,
    not only a number, and a table will happily write `ToolDurability:` and
    expect it to be somewhere to start writing. so the substitution happens
    before anything is read, on both halves, since the disable side leans on
    what the enable side named. */
    let named = named_pieces(header.iter().chain(&enable).chain(&disable));
    for line in enable.iter_mut().chain(disable.iter_mut()) {
        line.1 = swap_names(&line.1, &named);
    }

    Ok(Script {
        enable: read_half(&enable)?,
        disable: read_half(&disable)?,
    })
}

fn named_pieces<'a>(lines: impl Iterator<Item = &'a (usize, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = lines
        .filter_map(|(_, raw)| match read_directive(raw.trim()) {
            Ok(Some(Directive::Define { name, value })) => Some((name, value)),
            _ => None,
        })
        .filter(|(name, value)| !name.is_empty() && !value.contains(name.as_str()))
        .collect();
    // longest first so one name cannot eat the front of another
    out.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    out
}

fn swap_names(line: &str, named: &[(String, String)]) -> String {
    if matches!(
        read_directive(line.trim()),
        Ok(Some(Directive::Define { .. }))
    ) {
        return line.to_string();
    }
    let whole = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = line.to_string();
    for (name, value) in named {
        let mut built = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(name.as_str()) {
            let before = rest[..at].chars().next_back();
            let after = rest[at + name.len()..].chars().next();
            built.push_str(&rest[..at]);
            built.push_str(if before.is_some_and(whole) || after.is_some_and(whole) {
                name
            } else {
                value
            });
            rest = &rest[at + name.len()..];
        }
        built.push_str(rest);
        out = built;
    }
    out
}

fn read_half(lines: &[(usize, String)]) -> Result<Half> {
    let mut half = Half::default();
    let mut depth = 0usize;
    let mut blocked = false;
    let mut section: Option<Section> = None;
    let inner = declared_labels(lines);

    for (line, raw) in lines {
        /* three kinds of comment and whichever opens first wins. `{ }` is
        cheat engine's own, `/* */` came along later, `//` runs to the end of
        the line. doing them in a fixed order means a `{` inside a `//`
        comment opens a block that swallows the rest of the script, which is
        how one table lost forty lines. */
        let mut kept = String::new();
        let mut rest = raw.as_str();
        loop {
            if depth > 0 {
                match rest.find('}') {
                    Some(at) => {
                        depth -= 1;
                        rest = &rest[at + 1..];
                    }
                    None => break,
                }
            } else if blocked {
                match rest.find("*/") {
                    Some(at) => {
                        rest = &rest[at + 2..];
                        blocked = false;
                    }
                    None => break,
                }
            } else {
                let brace = rest.find('{');
                let block = rest.find("/*");
                let ends = rest.find("//");
                let first = [brace, block, ends].into_iter().flatten().min();
                let Some(at) = first else {
                    kept.push_str(rest);
                    break;
                };
                kept.push_str(&rest[..at]);
                if Some(at) == ends {
                    break;
                }
                blocked = Some(at) == block;
                depth += usize::from(Some(at) == brace);
                rest = &rest[at + if blocked { 2 } else { 1 }..];
            }
        }

        let trimmed = kept.trim();
        if trimmed.is_empty() {
            continue;
        }

        // a directive in the middle of a cave does not end it. they are all
        // hoisted and run first anyway, and closing the section there left
        // everything under it with nowhere to go
        if let Some(directive) = read_directive(trimmed)? {
            half.directives.push(directive);
            continue;
        }

        // `dbee(aob_hascontent)` and `monoTailCave32(1,"AIAir:Start",5)` come
        // from lua a table author had installed. saying so beats complaining
        // that it is not an instruction, which it was never meant to be. mono
        // is worth naming on its own: it means a unity game, not a bad table
        if let Some(name) = plugin_call(trimmed) {
            if is_mono(&name) {
                return Err(AaError::NeedsMono(name));
            }
            return Err(AaError::NeedsPlugin(name));
        }

        if let Some(name) = anchor_label(trimmed) {
            if !inner.contains(&name) && (section.is_none() || somewhere_real(&name, lines)) {
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

/* `end:` on its own in the middle of a cave is a branch target, not somewhere
to start writing, and plenty of scripts never declare it. splitting the
section there loses every `@f` that was meant to land on it and leaves an
anchor pointing at a symbol nothing defines.

so a name only starts a new section if the script says where it is: an
offset, a scan or an alloc that named it, or a module. */
fn somewhere_real(name: &str, lines: &[(usize, String)]) -> bool {
    // a dot or a colon in it makes it a module or a mono name, both of which
    // are somewhere rather than something to jump to
    let (head, offset) = split_offset(name);
    if offset.is_some() || head.contains(['.', ':']) || name.trim_start().starts_with(['"', '[']) {
        return true;
    }
    lines
        .iter()
        .any(|(_, raw)| match read_directive(raw.trim()) {
            Ok(Some(
                Directive::AobScanModule { symbol, .. }
                | Directive::AobScan { symbol, .. }
                | Directive::Alloc { symbol, .. }
                | Directive::Assert { symbol, .. }
                | Directive::RegisterSymbol(symbol)
                // a disable half that unregisters a name is saying it was an
                // address all along, not a branch target inside a cave
                | Directive::UnregisterSymbol(symbol),
            )) => symbol == head,
            _ => false,
        })
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

fn banned_call(raw: &str) -> Option<&'static str> {
    let mut line = raw.trim();
    if let Some(at) = line.find("//") {
        line = line[..at].trim();
    }
    let head = line.split('(').next()?.trim().to_ascii_lowercase();
    crate::safety::BANNED
        .iter()
        .find(|banned| **banned == head)
        .copied()
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
    // the last one, since a name lifted from a c++ binary has colons of its own
    let at = text.rfind(':')?;
    let mut name = text[..at].trim();
    if !text[at + 1..].trim().is_empty() {
        return None;
    }
    // "Terraria.Item::Prefix+2147)", a bracket somebody forgot to open
    while name.ends_with(')') && name.matches('(').count() < name.matches(')').count() {
        name = name[..name.len() - 1].trim_end();
    }

    // "god+07:" and "game.exe+1A2B3C:" are both somewhere to start writing, and
    // tables use them as freely as a plain label. anything cleverer than one
    // offset, like "aob+(DWORD)[aob+03]+07:", is left for a person to look at
    // `(DWORD)[tinkerUnlock+17]:` says how wide the pointer is, and where to
    // write is all that is left of it once that comes off
    let name = &name
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(')'))
        .filter(|(kind, _)| freeplay_asm::operand::size_keyword(kind).is_some())
        .map_or(name.to_string(), |(_, rest)| rest.trim().to_string());
    let (base, _) = split_offset(name);
    // a space only counts once the name has quotes round it or is a module
    let module = [".exe", ".dll", ".bin"]
        .iter()
        .any(|end| base.to_ascii_lowercase().contains(end));
    let spaced = name.trim_start().starts_with('"') || module;
    if base.is_empty()
        || !base
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || "_$[<".contains(c) || !c.is_ascii())
        || !base.chars().all(|c| {
            c.is_alphanumeric()
                || "_.-:$<>~=?!@'`[]+\"|&".contains(c)
                || !c.is_ascii()
                || (spaced && c == ' ')
        })
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

fn read_directive(raw: &str) -> Result<Option<Directive>> {
    // a note after the directive can have brackets of its own in it, and the
    // closing one is found from the right
    let text = match raw.find("//") {
        Some(at) => raw[..at].trim(),
        None => raw.trim(),
    };
    let Some(open) = text.find('(') else {
        return Ok(None);
    };
    let name = text[..open].trim().to_ascii_lowercase();
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok(None);
    }
    // one that was never closed still says what it wants
    let close = text
        .rfind(')')
        .filter(|at| *at > open)
        .unwrap_or(text.len());
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

// the mono helpers a table brings in from the ce mono plugin. all of them
// start mono, bar a couple of camelcase ones cheat engine ships
fn is_mono(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("mono") || lower.starts_with("il2cpp") || lower.starts_with("getmono")
}

fn plugin_call(text: &str) -> Option<String> {
    let line = text.trim();
    let (name, rest) = line.split_once('(')?;
    // readmem is put back as a `db` once there is a process to read from
    if !rest.ends_with(')') || name.is_empty() || name.eq_ignore_ascii_case("readmem") {
        return None;
    }
    let ok = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then(|| name.to_string())
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

    // a mono helper is a unity table, worth its own message rather than the
    // generic lua one
    #[test]
    fn a_mono_helper_is_refused_as_mono() {
        let source = "[ENABLE]\nmonoTailCave32(1,\"AIAir:Start\",5)\nnop\n[DISABLE]\n";
        let err = parse(source).unwrap_err();
        assert!(matches!(err, AaError::NeedsMono(_)), "{err:?}");
        assert!(err.to_string().contains("Mono"), "{err}");
    }

    #[test]
    fn a_non_mono_plugin_is_still_a_plain_plugin() {
        let source = "[ENABLE]\ndbee(aob_hascontent)\nnop\n[DISABLE]\n";
        let err = parse(source).unwrap_err();
        assert!(matches!(err, AaError::NeedsPlugin(_)), "{err:?}");
    }

    // real tables do this: a luacall on line one, above [ENABLE], where the
    // parser used to drop it on the floor without anybody checking it
    #[test]
    fn a_banned_call_above_enable_is_still_refused() {
        let source = "LuaCall(getMainForm().Panel4.Visible = false)\n[ENABLE]\nalloc(a,100)\na:\n  nop\n[DISABLE]\n";
        let why = parse(source).unwrap_err().to_string();
        assert!(why.contains("luacall"), "{why}");
    }

    #[test]
    fn a_commented_out_one_above_enable_is_fine() {
        let source = "//LuaCall(CheckVersion())\n[ENABLE]\nalloc(a,100)\na:\n  nop\n[DISABLE]\n";
        assert!(parse(source).is_ok());
    }

    #[test]
    fn an_ordinary_header_above_enable_is_left_alone() {
        let source = "{\n  Game : witcher2.exe\n  Author : somebody\n}\n[ENABLE]\nalloc(a,100)\na:\n  nop\n[DISABLE]\n";
        assert!(parse(source).is_ok());
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
