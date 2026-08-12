pub mod encode;
pub mod error;
pub mod operand;

use std::collections::HashMap;

pub use encode::Bits;
use encode::{Emitter, Rm};
pub use error::{AsmError, Result};
use operand::{parse_operand, split_operands, Class, Mem, Operand};

pub struct Assembled {
    pub bytes: Vec<u8>,
    pub labels: HashMap<String, u64>,
    // `db ?? ??` in a [DISABLE] means put back what was there, so these
    // offsets are filled in from the process rather than written over
    pub holes: Vec<usize>,
}

const ALU: &[(&str, u8, u8)] = &[
    ("add", 0x00, 0),
    ("or", 0x08, 1),
    ("adc", 0x10, 2),
    ("sbb", 0x18, 3),
    ("and", 0x20, 4),
    ("sub", 0x28, 5),
    ("xor", 0x30, 6),
    ("cmp", 0x38, 7),
];

const SHIFTS: &[(&str, u8)] = &[
    ("rol", 0),
    ("ror", 1),
    ("rcl", 2),
    ("rcr", 3),
    ("shl", 4),
    ("sal", 4),
    ("shr", 5),
    ("sar", 7),
];

const UNARY: &[(&str, u8)] = &[("not", 2), ("neg", 3), ("mul", 4), ("div", 6), ("idiv", 7)];

const CONDITIONS: &[(&str, u8)] = &[
    ("o", 0),
    ("no", 1),
    ("b", 2),
    ("c", 2),
    ("nae", 2),
    ("ae", 3),
    ("nb", 3),
    ("nc", 3),
    ("e", 4),
    ("z", 4),
    ("ne", 5),
    ("nz", 5),
    ("be", 6),
    ("na", 6),
    ("a", 7),
    ("nbe", 7),
    ("s", 8),
    ("ns", 9),
    ("p", 10),
    ("pe", 10),
    ("np", 11),
    ("po", 11),
    ("l", 12),
    ("nge", 12),
    ("ge", 13),
    ("nl", 13),
    ("le", 14),
    ("ng", 14),
    ("g", 15),
    ("nle", 15),
];

const SSE_ARITH: &[(&str, Option<u8>, u8)] = &[
    ("addss", Some(0xF3), 0x58),
    ("addsd", Some(0xF2), 0x58),
    ("addps", None, 0x58),
    ("mulss", Some(0xF3), 0x59),
    ("mulsd", Some(0xF2), 0x59),
    ("mulps", None, 0x59),
    ("subss", Some(0xF3), 0x5C),
    ("subsd", Some(0xF2), 0x5C),
    ("subps", None, 0x5C),
    ("divss", Some(0xF3), 0x5E),
    ("divsd", Some(0xF2), 0x5E),
    ("divps", None, 0x5E),
    ("minss", Some(0xF3), 0x5D),
    ("minsd", Some(0xF2), 0x5D),
    ("minps", None, 0x5D),
    ("minpd", Some(0x66), 0x5D),
    ("maxss", Some(0xF3), 0x5F),
    ("maxsd", Some(0xF2), 0x5F),
    ("maxps", None, 0x5F),
    ("maxpd", Some(0x66), 0x5F),
    ("sqrtss", Some(0xF3), 0x51),
    ("sqrtsd", Some(0xF2), 0x51),
    ("sqrtps", None, 0x51),
    ("sqrtpd", Some(0x66), 0x51),
    ("rcpss", Some(0xF3), 0x53),
    ("rsqrtss", Some(0xF3), 0x52),
    ("addpd", Some(0x66), 0x58),
    ("mulpd", Some(0x66), 0x59),
    ("subpd", Some(0x66), 0x5C),
    ("divpd", Some(0x66), 0x5E),
    ("xorps", None, 0x57),
    ("xorpd", Some(0x66), 0x57),
    ("andps", None, 0x54),
    ("andpd", Some(0x66), 0x54),
    ("andnps", None, 0x55),
    ("andnpd", Some(0x66), 0x55),
    ("orps", None, 0x56),
    ("orpd", Some(0x66), 0x56),
    ("comiss", None, 0x2F),
    ("comisd", Some(0x66), 0x2F),
    ("ucomiss", None, 0x2E),
    ("ucomisd", Some(0x66), 0x2E),
    ("cvtss2sd", Some(0xF3), 0x5A),
    ("cvtsd2ss", Some(0xF2), 0x5A),
    ("cvtdq2ps", None, 0x5B),
    ("cvtps2pd", None, 0x5A),
    ("cvtpd2ps", Some(0x66), 0x5A),
    ("cvtps2dq", Some(0x66), 0x5B),
    ("cvttps2dq", Some(0xF3), 0x5B),
    ("cvtdq2pd", Some(0xF3), 0xE6),
    ("cvtpd2dq", Some(0xF2), 0xE6),
    ("cvttpd2dq", Some(0x66), 0xE6),
    ("unpcklps", None, 0x14),
    ("unpckhps", None, 0x15),
    ("unpcklpd", Some(0x66), 0x14),
    ("unpckhpd", Some(0x66), 0x15),
    ("movhlps", None, 0x12),
    ("movlhps", None, 0x16),
    ("pxor", Some(0x66), 0xEF),
    ("pand", Some(0x66), 0xDB),
    ("pandn", Some(0x66), 0xDF),
    ("por", Some(0x66), 0xEB),
    ("paddb", Some(0x66), 0xFC),
    ("paddw", Some(0x66), 0xFD),
    ("paddd", Some(0x66), 0xFE),
    ("paddq", Some(0x66), 0xD4),
    ("psubb", Some(0x66), 0xF8),
    ("psubw", Some(0x66), 0xF9),
    ("psubd", Some(0x66), 0xFA),
    ("psubq", Some(0x66), 0xFB),
    ("pmullw", Some(0x66), 0xD5),
    ("pmuludq", Some(0x66), 0xF4),
    ("pcmpeqb", Some(0x66), 0x74),
    ("pcmpeqw", Some(0x66), 0x75),
    ("pcmpeqd", Some(0x66), 0x76),
    ("haddps", Some(0xF2), 0x7C),
    ("haddpd", Some(0x66), 0x7C),
    ("hsubps", Some(0xF2), 0x7D),
    ("hsubpd", Some(0x66), 0x7D),
    ("addsubps", Some(0xF2), 0xD0),
    ("packssdw", Some(0x66), 0x6B),
    ("packsswb", Some(0x66), 0x63),
    ("packuswb", Some(0x66), 0x67),
    ("punpcklbw", Some(0x66), 0x60),
    ("punpcklwd", Some(0x66), 0x61),
    ("punpckhbw", Some(0x66), 0x68),
    ("punpckhwd", Some(0x66), 0x69),
    ("punpckhdq", Some(0x66), 0x6A),
    ("punpckldq", Some(0x66), 0x62),
    ("punpcklqdq", Some(0x66), 0x6C),
    ("punpckhqdq", Some(0x66), 0x6D),
];

// the ones that end in a selector byte
const SSE_IMM8: &[(&str, Option<u8>, u8)] = &[
    ("shufps", None, 0xC6),
    ("shufpd", Some(0x66), 0xC6),
    ("pshufd", Some(0x66), 0x70),
    ("pshuflw", Some(0xF2), 0x70),
    ("pshufhw", Some(0xF3), 0x70),
    ("cmpps", None, 0xC2),
    ("cmpss", Some(0xF3), 0xC2),
    ("cmpsd", Some(0xF2), 0xC2),
    ("cmppd", Some(0x66), 0xC2),
    ("pshufw", None, 0x70),
];

// the sse4 ones, which sit behind a longer escape
const SSE_38: &[(&str, u8, u8)] = &[
    ("roundss", 0x3A, 0x0A),
    ("roundsd", 0x3A, 0x0B),
    ("roundps", 0x3A, 0x08),
    ("roundpd", 0x3A, 0x09),
    ("blendps", 0x3A, 0x0C),
    ("blendpd", 0x3A, 0x0D),
    ("palignr", 0x3A, 0x0F),
    ("insertps", 0x3A, 0x21),
];

const BARE: &[(&str, &[u8])] = &[
    ("rdtsc", &[0x0F, 0x31]),
    ("rdtscp", &[0x0F, 0x01, 0xF9]),
    ("cpuid", &[0x0F, 0xA2]),
    ("ud2", &[0x0F, 0x0B]),
    ("pause", &[0xF3, 0x90]),
    ("sahf", &[0x9E]),
    ("lahf", &[0x9F]),
    ("clc", &[0xF8]),
    ("stc", &[0xF9]),
    ("cmc", &[0xF5]),
    ("cld", &[0xFC]),
    ("std", &[0xFD]),
    ("xlat", &[0xD7]),
    ("cbw", &[0x66, 0x98]),
    ("cwde", &[0x98]),
    ("cdqe", &[0x48, 0x98]),
    ("movsb", &[0xA4]),
    ("movsw", &[0x66, 0xA5]),
    ("movsd", &[0xA5]),
    ("movsq", &[0x48, 0xA5]),
    ("stosb", &[0xAA]),
    ("stosw", &[0x66, 0xAB]),
    ("stosd", &[0xAB]),
    ("stosq", &[0x48, 0xAB]),
    ("lodsb", &[0xAC]),
    ("lodsw", &[0x66, 0xAD]),
    ("lodsd", &[0xAD]),
    ("lodsq", &[0x48, 0xAD]),
    ("scasb", &[0xAE]),
    ("scasd", &[0xAF]),
    ("cmpsb", &[0xA6]),
    ("emms", &[0x0F, 0x77]),
    ("outsd", &[0x6F]),
    ("outsb", &[0x6E]),
    ("insb", &[0x6C]),
    ("insd", &[0x6D]),
];

enum Item {
    Label(String),
    Instruction {
        line: usize,
        text: String,
    },
    Data {
        line: usize,
        width: usize,
        text: String,
    },
    Nop {
        count: usize,
    },
    Zero(usize),
    Align(usize),
}

pub fn assemble(
    source: &str,
    origin: u64,
    bits: Bits,
    externals: &HashMap<String, u64>,
) -> Result<Assembled> {
    let items = read_items(source)?;
    let mut emitter = Emitter::new(bits, origin);
    let mut labels: HashMap<String, u64> = HashMap::new();

    for item in &items {
        match item {
            Item::Label(name) => {
                if labels.contains_key(name) {
                    return Err(AsmError::DuplicateSymbol(name.clone()));
                }
                labels.insert(name.clone(), emitter.here());
            }
            Item::Nop { count } => {
                for _ in 0..*count {
                    emitter.byte(0x90);
                }
            }
            Item::Zero(count) => {
                for _ in 0..*count {
                    emitter.byte(0);
                }
            }
            Item::Align(to) => {
                while (emitter.here() as usize) % to != 0 {
                    emitter.byte(0x90);
                }
            }
            Item::Data { line, width, text } => {
                emit_data(&mut emitter, *width, text).map_err(|e| at(*line, e))?;
            }
            Item::Instruction { line, text } => {
                instruction(&mut emitter, text).map_err(|e| at(*line, e))?;
                emitter.close_relative_fixups();
            }
        }
    }

    let Emitter {
        mut bytes,
        fixups,
        holes,
        ..
    } = emitter;

    for fixup in &fixups {
        let look = |name: &str| labels.get(name).or_else(|| externals.get(name)).copied();

        // `jae FalloutNV.exe+39040D` and `mov eax,[base+1C]`. cheat engine
        // does the arithmetic in the operand rather than declaring a symbol
        // for every address it wants, and sometimes twice over
        let target = resolve(&fixup.symbol, &look)
            .ok_or_else(|| AsmError::UndefinedSymbol(fixup.symbol.clone()))?;

        let value = match fixup.relative_to_end {
            Some(end) => {
                let from = origin + end as u64;
                let delta = (target as i64 + fixup.addend) - from as i64;
                if fixup.width == 4 && !(-(1i64 << 31)..(1i64 << 31)).contains(&delta) {
                    return Err(AsmError::OutOfRange { from, target });
                }
                delta
            }
            None => target as i64 + fixup.addend,
        };

        let slice = &mut bytes[fixup.at..fixup.at + fixup.width];
        match fixup.width {
            1 => slice.copy_from_slice(&(value as i8).to_le_bytes()),
            2 => slice.copy_from_slice(&(value as i16).to_le_bytes()),
            4 => slice.copy_from_slice(&(value as i32).to_le_bytes()),
            8 => slice.copy_from_slice(&value.to_le_bytes()),
            _ => {}
        }
    }

    Ok(Assembled {
        bytes,
        labels,
        holes,
    })
}

fn at(line: usize, source: AsmError) -> AsmError {
    AsmError::At {
        line,
        source: Box::new(source),
    }
}

fn is_hint(word: &str) -> bool {
    matches!(
        word.trim().to_ascii_lowercase().as_str(),
        "short" | "near" | "far" | "long"
    )
}

fn read_items(source: &str) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    // /* ... */ can run over several lines, so it cannot be stripped a line
    // at a time like the others
    let mut commented = false;

    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let mut text = raw;

        // whichever comes first wins, or a `/*` sitting inside a `//` comment
        // opens a block that never closes
        let mut kept = String::new();
        loop {
            if commented {
                match text.find("*/") {
                    Some(at) => {
                        text = &text[at + 2..];
                        commented = false;
                    }
                    None => break,
                }
            } else {
                let block = text.find("/*");
                let ends = text.find("//").into_iter().chain(text.find(';')).min();
                match (block, ends) {
                    (Some(a), b) if b.is_none_or(|b| a < b) => {
                        kept.push_str(&text[..a]);
                        text = &text[a + 2..];
                        commented = true;
                    }
                    (_, Some(b)) => {
                        kept.push_str(&text[..b]);
                        break;
                    }
                    _ => {
                        kept.push_str(text);
                        break;
                    }
                }
            }
        }
        let holding = kept;
        let text = holding.trim();
        if text.is_empty() {
            continue;
        }

        let mut rest = text;
        while let Some(colon) = label_colon(rest) {
            let name = rest[..colon].trim().to_string();
            items.push(Item::Label(name));
            rest = rest[colon + 1..].trim();
        }
        // a line with nothing on it but a number is a slip of the keyboard.
        // real data says `db 4`, and writing the byte anyway would land in the
        // middle of the code a jump was aimed at
        if rest.is_empty() || (operand::is_number(rest) && !rest.contains(char::is_whitespace)) {
            continue;
        }

        // cheat engine writes `jmp!near foo` when it wants the long form. the
        // reach is worked out from the distance here, so the hint is noise
        let rest = match rest.split_once('!') {
            Some((head, tail)) if is_hint(tail.split_whitespace().next().unwrap_or("")) => {
                format!(
                    "{head} {}",
                    tail.split_once(char::is_whitespace).map_or("", |x| x.1)
                )
            }
            _ => rest.to_string(),
        };
        let rest = rest.trim();

        // `align,10` puts a comma where the space belongs
        let (head, tail) = match rest
            .find(char::is_whitespace)
            .into_iter()
            .chain(rest.find(','))
            .min()
        {
            Some(at) => (&rest[..at], rest[at + 1..].trim()),
            None => (rest, ""),
        };
        let head_lower = head.to_ascii_lowercase();

        // `jmp short foo`, `call far bar`. same story: a hint about the
        // encoding that this picks for itself
        let tail = match tail.split_once(char::is_whitespace) {
            Some((first, rest)) if is_hint(first) => rest.trim(),
            _ => tail,
        };
        let rest: &str = &if tail.is_empty() {
            head.to_string()
        } else {
            format!("{head} {tail}")
        };

        match head_lower.as_str() {
            "db" | "byte" => items.push(Item::Data {
                line,
                width: 1,
                text: tail.to_string(),
            }),
            "dw" | "word" => items.push(Item::Data {
                line,
                width: 2,
                text: tail.to_string(),
            }),
            "dd" | "dword" => items.push(Item::Data {
                line,
                width: 4,
                text: tail.to_string(),
            }),
            "dq" | "qword" => items.push(Item::Data {
                line,
                width: 8,
                text: tail.to_string(),
            }),
            "nop" if !tail.is_empty() => items.push(Item::Nop {
                count: operand::number(tail).unwrap_or(1).max(0) as usize,
            }),
            "resb" | "resw" | "resd" | "resq" => {
                let each = match head_lower.as_str() {
                    "resb" => 1,
                    "resw" => 2,
                    "resd" => 4,
                    _ => 8,
                };
                items.push(Item::Zero(
                    each * operand::number(tail).unwrap_or(1).max(0) as usize,
                ))
            }
            "align" => items.push(Item::Align(
                operand::number(tail).unwrap_or(1).max(1) as usize
            )),
            "aligndq" => items.push(Item::Align(16)),
            _ => items.push(Item::Instruction {
                line,
                text: rest.to_string(),
            }),
        }
    }

    name_the_anonymous(&mut items);
    Ok(items)
}

fn resolve(symbol: &str, look: &impl Fn(&str) -> Option<u64>) -> Option<u64> {
    if let Some(value) = look(symbol) {
        return Some(value);
    }
    if let (name, Some(extra)) = operand::split_offset(symbol) {
        let step = operand::number(extra).ok()?;
        return Some(resolve(name, look)?.wrapping_add_signed(step));
    }
    // `dd iEnableGM - MyCode`, which is how a script asks how long something is
    let at = symbol[1..].rfind(['+', '-']).map(|at| at + 1)?;
    let (left, right) = (&symbol[..at], &symbol[at + 1..]);
    let (a, b) = (resolve(left.trim(), look)?, resolve(right.trim(), look)?);
    Some(if symbol[at..].starts_with('-') {
        a.wrapping_sub(b)
    } else {
        a.wrapping_add(b)
    })
}

// from the right, because a mono name like `CharacterInventory:get_TotalWeight`
// has colons of its own and only the last one ends the label
fn label_colon(text: &str) -> Option<usize> {
    let mut at = text.len();
    while let Some(found) = text[..at].rfind(':') {
        at = found;
        // the `::` in a name lifted out of a c++ binary
        if text[at + 1..].starts_with(':') || text[..at].ends_with(':') {
            continue;
        }
        let name = text[..at].trim();
        // `@@:` is cheat engine's anonymous label. it gets a real name below.
        // a plain number is one too, and unreachable, but it is not code
        let named = operand::is_identifier(name)
            || name == ANON
            || name.chars().all(|c| c.is_alphanumeric());
        if !name.is_empty() && named {
            return Some(at);
        }
    }
    None
}

const ANON: &str = "@@";

/* cheat engine lets you drop `@@:` anywhere and jump to it with `@f` for the
next one forward or `@b` for the last one back, so a script can be written
without inventing a name for every branch. two thousand of ours use it.

each `@@:` gets a name of its own here, then every `@f` and `@b` is pointed
at the right one. it has to happen after the whole list is read, because a
forward reference names a label that has not been seen yet.

six hundred scripts jump `@f` at a named label with no `@@:` anywhere ahead
of them, which cheat engine is happy with, so a named label counts too when
there is no anonymous one that way. */
fn name_the_anonymous(items: &mut [Item]) {
    let mut count = 0usize;
    let mut spots: Vec<(usize, String, bool)> = Vec::new();

    for (at, item) in items.iter_mut().enumerate() {
        if let Item::Label(name) = item {
            if name == ANON {
                *name = format!("__anon{count}");
                count += 1;
                spots.push((at, name.clone(), true));
            } else {
                spots.push((at, name.clone(), false));
            }
        }
    }
    if spots.is_empty() {
        return;
    }

    for (at, item) in items.iter_mut().enumerate() {
        let text = match item {
            Item::Instruction { text, .. } | Item::Data { text, .. } => text,
            _ => continue,
        };
        if !text.contains('@') {
            continue;
        }

        let forward = nearest(spots.iter().filter(|(spot, ..)| *spot > at));
        let back = nearest(spots.iter().rev().filter(|(spot, ..)| *spot < at));

        *text = swap_anon(text, forward, back);
    }
}

// the closest `@@:` that way if there is one, else the closest label of any kind
fn nearest<'a>(run: impl Iterator<Item = &'a (usize, String, bool)>) -> Option<&'a str> {
    let mut first = None;
    for (_, name, anonymous) in run {
        if *anonymous {
            return Some(name);
        }
        first.get_or_insert(name.as_str());
    }
    first
}

fn swap_anon(text: &str, forward: Option<&str>, back: Option<&str>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some((at, c)) = chars.next() {
        if c != '@' {
            out.push(c);
            continue;
        }
        let next = text[at + 1..].chars().next();
        // it has to be the whole word, or a symbol like `@foo` gets mangled
        let after = text[at + 2..].chars().next();
        let alone = after.is_none_or(|c| !c.is_alphanumeric() && c != '_');

        match next {
            Some('f' | 'F') if alone && forward.is_some() => {
                out.push_str(forward.unwrap());
                chars.next();
            }
            Some('b' | 'B') if alone && back.is_some() => {
                out.push_str(back.unwrap());
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

fn emit_data(emitter: &mut Emitter, width: usize, text: &str) -> Result<()> {
    for piece in split_data(text) {
        // `db 90 90 90 /mov [rcx],r11`, a note written with one slash instead
        // of two, so everything from there on is not data
        if piece.starts_with('/') {
            break;
        }
        // a stray brace from the block of original code above it. a bracket is
        // left alone unless nothing opened it, since `(float)1` needs both
        let mut piece = piece.trim().trim_matches(['{', '}']);
        while piece.ends_with(')') && piece.matches('(').count() < piece.matches(')').count() {
            piece = piece[..piece.len() - 1].trim_end();
        }
        if piece.is_empty() {
            continue;
        }

        // `db 'GAME OVER'`, which is a run of bytes rather than a number, and
        // may be longer than anything a number would fit in
        if let Some(inner) = quoted(piece, width) {
            emitter.bytes_from(inner.as_bytes());
            continue;
        }
        if piece == "''" {
            continue;
        }
        // a wildcard, left over from pasting an aob in as the original bytes.
        // nothing to write, so whatever is there stays
        if piece.contains('?') || (piece.contains('*') && piece.len() <= 2) {
            for _ in 0..width {
                emitter.holes.push(emitter.bytes.len());
                emitter.byte(0);
            }
            continue;
        }

        if let Some(rest) = strip_cast(piece, "float") {
            let value: f32 = rest.parse().map_err(|_| AsmError::Operand(piece.into()))?;
            emitter.dword(value.to_bits());
            continue;
        }
        if let Some(rest) = strip_cast(piece, "double") {
            let value: f64 = rest.parse().map_err(|_| AsmError::Operand(piece.into()))?;
            emitter.qword(value.to_bits());
            continue;
        }

        if operand::is_number(piece) {
            emitter.immediate(operand::number(piece)?, width);
        } else if operand::is_identifier(piece) || operand::is_identifier_with_offset(piece) {
            emitter.fixup(piece, width, 0, false);
        } else if let Some(plain) = operand::unquote(piece) {
            emitter.fixup(&plain, width, 0, false);
        } else {
            return Err(AsmError::Operand(piece.to_string()));
        }
    }
    Ok(())
}

/* two operands run together because the comma between them was never typed,
which cheat engine forgives and plenty of tables rely on. only tried once the
operand has already failed to parse whole, so `dword ptr [esi]` is safe. */
fn tidy(parts: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        // `cmp dword ptr,[esi+08],0`, where the comma is one word too early
        if let Some(last) = out.last_mut().filter(|last| just_a_size(last)) {
            last.push(' ');
            last.push_str(&part);
            continue;
        }
        if just_a_size(&part) || parse_operand(&part).is_ok() {
            out.push(part);
            continue;
        }
        match torn(&part) {
            Some((a, b)) => out.extend([a, b]),
            // `mov [ecx+2C]+[ecx+44],1`. only the first read is an operand and
            // the rest of it never meant anything
            None => out.push(match upto_bracket(&part) {
                Some(head) => head,
                None => part,
            }),
        }
    }
    out
}

fn upto_bracket(part: &str) -> Option<String> {
    let at = part.find(']')?;
    let head = part[..=at].trim().to_string();
    parse_operand(&head).is_ok().then_some(head)
}

fn just_a_size(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    let head = lower
        .strip_suffix("ptr")
        .or_else(|| lower.strip_suffix("prt"))
        .unwrap_or(&lower);
    operand::size_keyword(head.trim()).is_some()
}

fn torn(part: &str) -> Option<(String, String)> {
    part.char_indices()
        .filter(|(at, c)| c.is_whitespace() || *c == ']' || (*c == '[' && *at > 0))
        .find_map(|(at, c)| {
            let (a, b) = part.split_at(if c == ']' { at + 1 } else { at });
            let (a, b) = (a.trim(), b.trim());
            (!b.is_empty() && parse_operand(a).is_ok() && parse_operand(b).is_ok())
                .then(|| (a.to_string(), b.to_string()))
        })
}

// in a db it is always a run of bytes. anywhere wider it is a packed tag like
// 'Bott', unless it is too long to be one
fn quoted(text: &str, width: usize) -> Option<&str> {
    let inner = text.strip_prefix('\'')?.strip_suffix('\'')?;
    ((width == 1 || inner.len() > 8) && !inner.contains('\'')).then_some(inner)
}

// commas or spaces, either way, except inside a string where neither counts
fn split_data(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut inside = false;

    for ch in text.chars() {
        match ch {
            '\'' => {
                inside = !inside;
                current.push(ch);
            }
            ',' | ' ' | '\t' if !inside => {
                if !current.trim().is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }

    // `dd iEnableGM - MyCode` is one value with spaces round the sign
    let mut joined: Vec<String> = Vec::new();
    for piece in out {
        let hanging = joined.last().is_some_and(|last: &String| {
            last.ends_with(['+', '-']) || matches!(piece.as_str(), "+" | "-")
        });
        match joined.last_mut().filter(|_| hanging) {
            Some(last) => last.push_str(&piece),
            None => joined.push(piece),
        }
    }
    joined
}

fn strip_cast<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    let prefix = format!("({name})");
    if lower.starts_with(&prefix) {
        Some(text[prefix.len()..].trim())
    } else {
        None
    }
}

fn to_rm(op: &Operand) -> Result<Rm> {
    match op {
        Operand::Reg(r) => Ok(Rm::Reg(*r)),
        Operand::Mem(m) => Ok(Rm::Mem(m.clone())),
        Operand::Symbol(name) => Ok(Rm::Mem(Mem {
            symbol: Some(name.clone()),
            ..Default::default()
        })),
        Operand::Imm(_) => Err(AsmError::Operand("immediate where memory expected".into())),
    }
}

fn operand_size(ops: &[Operand], default: usize) -> usize {
    for op in ops {
        match op {
            Operand::Reg(r) if r.is_gpr() => return r.size(),
            Operand::Mem(m) => {
                if let Some(size) = m.size {
                    return size;
                }
            }
            _ => {}
        }
    }
    default
}

fn instruction(emitter: &mut Emitter, text: &str) -> Result<()> {
    // `cvtsd2ss,xmm5,xmm0` with a comma where the space should be
    let cut = text
        .find(char::is_whitespace)
        .into_iter()
        .chain(text.find(','))
        .min();
    let (mnemonic, tail) = match cut {
        Some(at) => (&text[..at], text[at + 1..].trim()),
        None => (text, ""),
    };
    let mut name = mnemonic.to_ascii_lowercase();
    // `jmp1 newmem`, which is a jump with a stray keystroke on the end of it
    if !tail.is_empty() && !known(&name) {
        let shorter = name.trim_end_matches(|c: char| c.is_ascii_digit());
        if shorter.len() < name.len() && known(shorter) {
            name = shorter.to_string();
        }
    }
    // `mov //ecx,[esi+10AC]`, where the operands were commented out and the
    // mnemonic left standing. it cannot mean anything on its own, so it goes
    if tail.is_empty() && needs_operands(&name) {
        return Ok(());
    }

    let read = |text: &str| {
        tidy(split_operands(text))
            .iter()
            .map(|p| parse_operand(p))
            .collect::<Result<Vec<_>>>()
    };
    let ops = match read(tail) {
        Ok(ops) => ops,
        // `jmp!near foo: c3 | 00*`, where a note about the target got written
        // straight onto the line without a comment marker in front of it
        Err(e) => match tail.split_once(':').map(|(head, _)| head.trim()) {
            Some(head) if !head.is_empty() => read(head).map_err(|_| e)?,
            _ => return Err(e),
        },
    };

    let default_size = emitter.bits.pointer().min(4);

    match name.as_str() {
        "nop" => {
            emitter.byte(0x90);
            Ok(())
        }
        "int3" => {
            emitter.byte(0xCC);
            Ok(())
        }
        "ret" | "retn" => {
            if ops.is_empty() {
                emitter.byte(0xC3);
            } else if let Operand::Imm(value) = ops[0] {
                emitter.byte(0xC2);
                emitter.word(value as u16);
            } else {
                return Err(bad(&name));
            }
            Ok(())
        }
        "leave" => {
            emitter.byte(0xC9);
            Ok(())
        }
        "cdq" => {
            emitter.byte(0x99);
            Ok(())
        }
        "cqo" => {
            emitter.byte(0x48);
            emitter.byte(0x99);
            Ok(())
        }
        "cwd" => {
            emitter.byte(0x66);
            emitter.byte(0x99);
            Ok(())
        }
        // the bare forms mean whatever the mode's default width is, which for
        // everything here is the same byte
        "pushf" | "pushfd" | "pushfq" => {
            emitter.byte(0x9C);
            Ok(())
        }
        "popf" | "popfd" | "popfq" => {
            emitter.byte(0x9D);
            Ok(())
        }
        "int" => {
            let which = ops
                .first()
                .and_then(|o| match o {
                    Operand::Imm(v) => Some(*v),
                    _ => None,
                })
                .ok_or_else(|| bad(&name))?;
            // int 3 has a one byte form of its own and debuggers expect it
            if which == 3 {
                emitter.byte(0xCC);
            } else {
                emitter.byte(0xCD);
                emitter.byte(which as u8);
            }
            Ok(())
        }
        "pushad" => {
            emitter.byte(0x60);
            Ok(())
        }
        "popad" => {
            emitter.byte(0x61);
            Ok(())
        }
        /* the registers and the flags in one go. x86 has an instruction for
        each half, x64 has neither, so there it is written out. rsp is left
        alone because pushing it is what moves it. */
        "pushall" | "popall" => {
            let up = name == "pushall";
            if emitter.bits == Bits::X86 {
                emitter.bytes_from(if up { &[0x60, 0x9C] } else { &[0x9D, 0x61] });
                return Ok(());
            }
            let order: [u8; 15] = [0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
            if up {
                emitter.byte(0x9C);
            }
            for num in if up { order } else { flipped(order) } {
                if num >= 8 {
                    emitter.byte(0x41);
                }
                emitter.byte(if up { 0x50 } else { 0x58 } + (num & 7));
            }
            if !up {
                emitter.byte(0x9D);
            }
            Ok(())
        }
        "mov" => mov(emitter, &ops, default_size),
        "lea" => {
            let reg = ops
                .first()
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(ops.get(1).ok_or_else(|| bad(&name))?)?;
            emitter.encode_reg_rm(&[0x8D], reg, &rm, reg.size())
        }
        "movzx" | "movsx" => {
            let reg = ops
                .first()
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(ops.get(1).ok_or_else(|| bad(&name))?)?;
            let source = rm.size_hint().unwrap_or(1);
            let base = if name == "movzx" { 0xB6 } else { 0xBE };
            let opcode = [0x0F, base + if source == 2 { 1 } else { 0 }];
            emitter.encode_reg_rm(&opcode, reg, &rm, reg.size())
        }
        // sign extend a dword into a qword. its own opcode rather than a
        // width of movsx, and the only one of the three that is x64 only
        "movsxd" => {
            let reg = ops
                .first()
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(ops.get(1).ok_or_else(|| bad(&name))?)?;
            emitter.encode_reg_rm(&[0x63], reg, &rm, reg.size())
        }
        "push" => push(emitter, &ops),
        "pop" => pop(emitter, &ops),
        "test" => {
            let size = operand_size(&ops, default_size);
            match (ops.first(), ops.get(1)) {
                (Some(dst), Some(Operand::Imm(value))) => {
                    let rm = to_rm(dst)?;
                    emitter.encode_rm(&[if size == 1 { 0xF6 } else { 0xF7 }], 0, &rm, size)?;
                    emitter.immediate(*value, if size == 1 { 1 } else { 4.min(size) });
                    Ok(())
                }
                (Some(dst), Some(Operand::Reg(reg))) => {
                    let rm = to_rm(dst)?;
                    emitter.encode_reg_rm(&[if size == 1 { 0x84 } else { 0x85 }], *reg, &rm, size)
                }
                _ => Err(bad(&name)),
            }
        }
        // either way round, the register goes in the reg field
        "xchg" => {
            let (reg, other) = match ops.first().and_then(Operand::as_reg) {
                Some(reg) => (reg, ops.get(1)),
                None => (
                    ops.get(1)
                        .and_then(Operand::as_reg)
                        .ok_or_else(|| bad(&name))?,
                    ops.first(),
                ),
            };
            let rm = to_rm(other.ok_or_else(|| bad(&name))?)?;
            let size = reg.size();
            emitter.encode_reg_rm(&[if size == 1 { 0x86 } else { 0x87 }], reg, &rm, size)
        }
        "inc" | "dec" => {
            let size = operand_size(&ops, default_size);
            let rm = to_rm(ops.first().ok_or_else(|| bad(&name))?)?;
            let digit = if name == "inc" { 0 } else { 1 };
            if emitter.bits == Bits::X86 {
                if let Rm::Reg(reg) = &rm {
                    if matches!(reg.class, Class::Gpr16 | Class::Gpr32) {
                        if size == 2 {
                            emitter.byte(0x66);
                        }
                        emitter.byte(if digit == 0 { 0x40 } else { 0x48 } + reg.low3());
                        return Ok(());
                    }
                }
            }
            emitter.encode_rm(&[if size == 1 { 0xFE } else { 0xFF }], digit, &rm, size)
        }
        "imul" => imul(emitter, &ops, default_size),
        "jmp" => jump(emitter, &ops, &[0xE9], 4),
        "call" => jump(emitter, &ops, &[0xE8], 2),
        // the only branch left with nothing but a byte of reach
        "loop" | "loope" | "loopz" | "loopne" | "loopnz" => {
            let Some(Operand::Symbol(target)) = ops.first() else {
                return Err(bad(&name));
            };
            emitter.byte(match name.as_str() {
                "loop" => 0xE2,
                "loope" | "loopz" => 0xE1,
                _ => 0xE0,
            });
            emitter.fixup(target, 1, 0, true);
            Ok(())
        }
        "bt" | "bts" | "btr" | "btc" => {
            let digit = match name.as_str() {
                "bt" => 4,
                "bts" => 5,
                "btr" => 6,
                _ => 7,
            };
            let size = operand_size(&ops, default_size);
            let rm = to_rm(ops.first().ok_or_else(|| bad(&name))?)?;
            match ops.get(1) {
                Some(Operand::Imm(value)) => {
                    emitter.encode_rm(&[0x0F, 0xBA], digit, &rm, size)?;
                    emitter.byte(*value as u8);
                    Ok(())
                }
                Some(Operand::Reg(reg)) => {
                    let opcode = [0x0F, 0xA3 + (digit - 4) * 8];
                    emitter.encode_reg_rm(&opcode, *reg, &rm, size)
                }
                _ => Err(bad(&name)),
            }
        }
        "bsf" | "bsr" | "popcnt" | "lzcnt" | "tzcnt" => {
            let reg = ops
                .first()
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(ops.get(1).ok_or_else(|| bad(&name))?)?;
            if name != "bsf" && name != "bsr" {
                emitter.byte(0xF3);
            }
            let opcode = match name.as_str() {
                "bsf" | "tzcnt" => 0xBC,
                "popcnt" => 0xB8,
                _ => 0xBD,
            };
            emitter.encode_reg_rm(&[0x0F, opcode], reg, &rm, reg.size())
        }
        "xadd" => {
            let reg = ops
                .get(1)
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(ops.first().ok_or_else(|| bad(&name))?)?;
            let size = reg.size();
            emitter.encode_reg_rm(&[0x0F, if size == 1 { 0xC0 } else { 0xC1 }], reg, &rm, size)
        }
        "cmpxchg" => {
            let reg = ops
                .get(1)
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(ops.first().ok_or_else(|| bad(&name))?)?;
            let size = reg.size();
            emitter.encode_reg_rm(&[0x0F, if size == 1 { 0xB0 } else { 0xB1 }], reg, &rm, size)
        }
        "movbe" => {
            if let Some(reg) = ops.first().and_then(Operand::as_reg) {
                let rm = to_rm(ops.get(1).ok_or_else(|| bad(&name))?)?;
                return emitter.encode_reg_rm(&[0x0F, 0x38, 0xF0], reg, &rm, reg.size());
            }
            let source = ops
                .get(1)
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(&ops[0])?;
            emitter.encode_reg_rm(&[0x0F, 0x38, 0xF1], source, &rm, source.size())
        }
        "bswap" => {
            let reg = ops
                .first()
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            if reg.size() == 8 {
                emitter.byte(if reg.extended() { 0x49 } else { 0x48 });
            } else if reg.extended() {
                emitter.byte(0x41);
            }
            two(emitter, 0x0F, 0xC8 + reg.low3())
        }
        "enter" => {
            let (Some(Operand::Imm(space)), Some(Operand::Imm(level))) = (ops.first(), ops.get(1))
            else {
                return Err(bad(&name));
            };
            emitter.byte(0xC8);
            emitter.word(*space as u16);
            emitter.byte(*level as u8);
            Ok(())
        }
        "fxsave" | "fxrstor" | "ldmxcsr" | "stmxcsr" => {
            let digit = match name.as_str() {
                "fxsave" => 0,
                "fxrstor" => 1,
                "ldmxcsr" => 2,
                _ => 3,
            };
            let rm = to_rm(ops.first().ok_or_else(|| bad(&name))?)?;
            emitter.encode_rm(&[0x0F, 0xAE], digit, &rm, 4)
        }
        // no operands and no encoding to work out, just the bytes. movsd and
        // cmpsd are also sse, which is what they mean once they take operands
        _ if ops.is_empty() && BARE.iter().any(|(m, _)| *m == name) => {
            let (_, code) = BARE.iter().find(|(m, _)| *m == name).unwrap();
            emitter.bytes_from(code);
            Ok(())
        }
        // a prefix and then a whole other instruction
        "rep" | "repe" | "repz" | "repne" | "repnz" | "lock" => {
            emitter.byte(match name.as_str() {
                "lock" => 0xF0,
                "repne" | "repnz" => 0xF2,
                _ => 0xF3,
            });
            match tail.trim() {
                "" => Ok(()),
                rest => instruction(emitter, rest),
            }
        }
        _ => {
            if let Some((_, base, digit)) = ALU.iter().find(|(m, _, _)| *m == name) {
                return alu(emitter, &ops, *base, *digit, default_size, &name);
            }
            if let Some((_, digit)) = SHIFTS.iter().find(|(m, _)| *m == name) {
                return shift(emitter, &ops, *digit, default_size, &name);
            }
            if let Some((_, digit)) = UNARY.iter().find(|(m, _)| *m == name) {
                let size = operand_size(&ops, default_size);
                let rm = to_rm(ops.first().ok_or_else(|| bad(&name))?)?;
                return emitter.encode_rm(
                    &[if size == 1 { 0xF6 } else { 0xF7 }],
                    *digit,
                    &rm,
                    size,
                );
            }
            if let Some(rest) = name.strip_prefix('j') {
                if let Some((_, code)) = CONDITIONS.iter().find(|(c, _)| *c == rest) {
                    return jump(emitter, &ops, &[0x0F, 0x80 + code], 0xFF);
                }
            }
            if let Some(rest) = name.strip_prefix("set") {
                if let Some((_, code)) = CONDITIONS.iter().find(|(c, _)| *c == rest) {
                    let rm = to_rm(ops.first().ok_or_else(|| bad(&name))?)?;
                    return emitter.encode_rm(&[0x0F, 0x90 + code], 0, &rm, 1);
                }
            }
            if let Some(rest) = name.strip_prefix("cmov") {
                if let Some((_, code)) = CONDITIONS.iter().find(|(c, _)| *c == rest) {
                    let reg = ops
                        .first()
                        .and_then(Operand::as_reg)
                        .ok_or_else(|| bad(&name))?;
                    let rm = to_rm(ops.get(1).ok_or_else(|| bad(&name))?)?;
                    return emitter.encode_reg_rm(&[0x0F, 0x40 + code], reg, &rm, reg.size());
                }
            }
            if name.starts_with('f') {
                return x87(emitter, &name, &ops);
            }
            sse(emitter, &name, &ops)
        }
    }
}

fn needs_operands(name: &str) -> bool {
    ALU.iter().any(|(m, ..)| *m == name)
        || matches!(
            name,
            "mov" | "lea" | "test" | "xchg" | "movzx" | "movsx" | "movsxd" | "imul"
        )
}

// only the ones worth rescuing a typo for: a branch that goes somewhere
fn known(name: &str) -> bool {
    matches!(name, "jmp" | "call")
        || name
            .strip_prefix('j')
            .is_some_and(|rest| CONDITIONS.iter().any(|(c, _)| *c == rest))
}

fn flipped<const N: usize>(mut order: [u8; N]) -> [u8; N] {
    order.reverse();
    order
}

fn bad(mnemonic: &str) -> AsmError {
    AsmError::BadOperands {
        mnemonic: mnemonic.to_string(),
    }
}

fn mov(emitter: &mut Emitter, ops: &[Operand], default: usize) -> Result<()> {
    let (dst, src) = (
        ops.first().ok_or_else(|| bad("mov"))?,
        ops.get(1).ok_or_else(|| bad("mov"))?,
    );

    if let (Operand::Reg(a), Operand::Reg(b)) = (dst, src) {
        if matches!(a.class, Class::Xmm | Class::Ymm) || matches!(b.class, Class::Xmm | Class::Ymm)
        {
            return sse(emitter, "movaps", ops);
        }
    }

    match (dst, src) {
        (Operand::Reg(reg), Operand::Imm(value)) if reg.is_gpr() => {
            let size = reg.size();
            if size == 8 && !(-(1i64 << 31)..(1i64 << 31)).contains(value) {
                emitter.byte(if reg.extended() { 0x49 } else { 0x48 });
                emitter.byte(0xB8 + reg.low3());
                emitter.qword(*value as u64);
                return Ok(());
            }
            if size == 8 {
                let rm = Rm::Reg(*reg);
                emitter.encode_rm(&[0xC7], 0, &rm, size)?;
                emitter.dword(*value as u32);
                return Ok(());
            }
            if size == 2 {
                emitter.byte(0x66);
            }
            if reg.extended() {
                emitter.byte(0x41);
            } else if size == 1 && reg.class == Class::Gpr8Rex {
                emitter.byte(0x40);
            }
            emitter.byte(if size == 1 { 0xB0 } else { 0xB8 } + reg.low3());
            emitter.immediate(*value, size);
            Ok(())
        }
        (Operand::Reg(reg), Operand::Symbol(name)) if reg.is_gpr() => {
            let size = reg.size();
            let rm = Rm::Reg(*reg);
            emitter.encode_rm(&[if size == 1 { 0xC6 } else { 0xC7 }], 0, &rm, size)?;
            emitter.fixup(name, 4.min(size), 0, false);
            Ok(())
        }
        (_, Operand::Imm(value)) => {
            let size = operand_size(ops, default);
            let rm = to_rm(dst)?;
            emitter.encode_rm(&[if size == 1 { 0xC6 } else { 0xC7 }], 0, &rm, size)?;
            emitter.immediate(*value, if size == 1 { 1 } else { 4.min(size) });
            Ok(())
        }
        (_, Operand::Symbol(name)) => {
            let size = operand_size(ops, default);
            let rm = to_rm(dst)?;
            emitter.encode_rm(&[if size == 1 { 0xC6 } else { 0xC7 }], 0, &rm, size)?;
            emitter.fixup(name, 4.min(size), 0, false);
            Ok(())
        }
        (Operand::Reg(reg), source) => {
            let rm = to_rm(source)?;
            let size = reg.size();
            emitter.encode_reg_rm(&[if size == 1 { 0x8A } else { 0x8B }], *reg, &rm, size)
        }
        (target, Operand::Reg(reg)) => {
            let rm = to_rm(target)?;
            let size = reg.size();
            emitter.encode_reg_rm(&[if size == 1 { 0x88 } else { 0x89 }], *reg, &rm, size)
        }
        _ => Err(bad("mov")),
    }
}

fn alu(
    emitter: &mut Emitter,
    ops: &[Operand],
    base: u8,
    digit: u8,
    default: usize,
    name: &str,
) -> Result<()> {
    let dst = ops.first().ok_or_else(|| bad(name))?;
    let src = ops.get(1).ok_or_else(|| bad(name))?;
    let size = operand_size(ops, default);

    match src {
        Operand::Imm(value) => {
            let rm = to_rm(dst)?;
            if size != 1 && (-128..=127).contains(value) {
                emitter.encode_rm(&[0x83], digit, &rm, size)?;
                emitter.byte(*value as u8);
            } else {
                emitter.encode_rm(&[if size == 1 { 0x80 } else { 0x81 }], digit, &rm, size)?;
                emitter.immediate(*value, if size == 1 { 1 } else { 4.min(size) });
            }
            Ok(())
        }
        Operand::Reg(reg) => {
            let rm = to_rm(dst)?;
            emitter.encode_reg_rm(&[base + if size == 1 { 0 } else { 1 }], *reg, &rm, size)
        }
        // `cmp [esp+4],SomeLabel` wants the label's address as a number.
        // there is no memory to memory form to fall back on
        Operand::Symbol(symbol) if dst.as_mem().is_some() => {
            let rm = to_rm(dst)?;
            emitter.encode_rm(&[0x81], digit, &rm, size.max(4))?;
            emitter.fixup(symbol, 4, 0, false);
            Ok(())
        }
        _ => {
            let reg = dst.as_reg().ok_or_else(|| bad(name))?;
            let rm = to_rm(src)?;
            emitter.encode_reg_rm(&[base + if size == 1 { 2 } else { 3 }], reg, &rm, size)
        }
    }
}

fn shift(
    emitter: &mut Emitter,
    ops: &[Operand],
    digit: u8,
    default: usize,
    name: &str,
) -> Result<()> {
    let dst = ops.first().ok_or_else(|| bad(name))?;
    let size = operand_size(ops, default);
    let rm = to_rm(dst)?;

    match ops.get(1) {
        None => emitter.encode_rm(&[if size == 1 { 0xD0 } else { 0xD1 }], digit, &rm, size),
        Some(Operand::Imm(1)) => {
            emitter.encode_rm(&[if size == 1 { 0xD0 } else { 0xD1 }], digit, &rm, size)
        }
        Some(Operand::Imm(value)) => {
            emitter.encode_rm(&[if size == 1 { 0xC0 } else { 0xC1 }], digit, &rm, size)?;
            emitter.byte(*value as u8);
            Ok(())
        }
        Some(Operand::Reg(_)) => {
            emitter.encode_rm(&[if size == 1 { 0xD2 } else { 0xD3 }], digit, &rm, size)
        }
        _ => Err(bad(name)),
    }
}

fn imul(emitter: &mut Emitter, ops: &[Operand], default: usize) -> Result<()> {
    match ops.len() {
        1 => {
            let size = operand_size(ops, default);
            let rm = to_rm(&ops[0])?;
            emitter.encode_rm(&[if size == 1 { 0xF6 } else { 0xF7 }], 5, &rm, size)
        }
        // `imul rdx,4` is the three operand form with the destination
        // standing in for the source
        2 if matches!(ops[1], Operand::Imm(_)) => imul(
            emitter,
            &[ops[0].clone(), ops[0].clone(), ops[1].clone()],
            default,
        ),
        2 => {
            let reg = ops[0].as_reg().ok_or_else(|| bad("imul"))?;
            let rm = to_rm(&ops[1])?;
            emitter.encode_reg_rm(&[0x0F, 0xAF], reg, &rm, reg.size())
        }
        3 => {
            let reg = ops[0].as_reg().ok_or_else(|| bad("imul"))?;
            let rm = to_rm(&ops[1])?;
            // a symbol here is a number the fixups will fill in
            if let Operand::Symbol(name) = &ops[2] {
                emitter.encode_reg_rm(&[0x69], reg, &rm, reg.size())?;
                emitter.fixup(name, 4, 0, false);
                return Ok(());
            }
            let Operand::Imm(value) = ops[2] else {
                return Err(bad("imul"));
            };
            if (-128..=127).contains(&value) {
                emitter.encode_reg_rm(&[0x6B], reg, &rm, reg.size())?;
                emitter.byte(value as u8);
            } else {
                emitter.encode_reg_rm(&[0x69], reg, &rm, reg.size())?;
                emitter.dword(value as u32);
            }
            Ok(())
        }
        _ => Err(bad("imul")),
    }
}

fn push(emitter: &mut Emitter, ops: &[Operand]) -> Result<()> {
    match ops.first() {
        Some(Operand::Reg(reg)) if reg.is_gpr() => {
            if reg.extended() {
                emitter.byte(0x41);
            }
            emitter.byte(0x50 + reg.low3());
            Ok(())
        }
        Some(Operand::Imm(value)) => {
            if (-128..=127).contains(value) {
                emitter.byte(0x6A);
                emitter.byte(*value as u8);
            } else {
                emitter.byte(0x68);
                emitter.dword(*value as u32);
            }
            Ok(())
        }
        Some(Operand::Symbol(name)) => {
            emitter.byte(0x68);
            emitter.fixup(name, 4, 0, false);
            Ok(())
        }
        Some(other) => {
            let rm = to_rm(other)?;
            emitter.encode_rm(&[0xFF], 6, &rm, 4)
        }
        None => Err(bad("push")),
    }
}

fn pop(emitter: &mut Emitter, ops: &[Operand]) -> Result<()> {
    match ops.first() {
        Some(Operand::Reg(reg)) if reg.is_gpr() => {
            if reg.extended() {
                emitter.byte(0x41);
            }
            emitter.byte(0x58 + reg.low3());
            Ok(())
        }
        Some(other) => {
            let rm = to_rm(other)?;
            emitter.encode_rm(&[0x8F], 0, &rm, 4)
        }
        None => Err(bad("pop")),
    }
}

fn jump(emitter: &mut Emitter, ops: &[Operand], opcode: &[u8], indirect_digit: u8) -> Result<()> {
    match ops.first() {
        Some(Operand::Symbol(name)) => {
            emitter.bytes_from(opcode);
            emitter.fixup(name, 4, 0, true);
            Ok(())
        }
        Some(Operand::Imm(target)) => {
            emitter.bytes_from(opcode);
            let end = emitter.here() + 4;
            emitter.dword((*target - end as i64) as u32);
            Ok(())
        }
        Some(other) if indirect_digit != 0xFF => {
            let rm = to_rm(other)?;
            emitter.encode_rm(&[0xFF], indirect_digit, &rm, 4)
        }
        _ => Err(bad("jmp")),
    }
}

// the ones that are two fixed bytes and take nothing
const X87_BARE: &[(&str, u8, u8)] = &[
    ("fld1", 0xD9, 0xE8),
    ("fldl2t", 0xD9, 0xE9),
    ("fldl2e", 0xD9, 0xEA),
    ("fldpi", 0xD9, 0xEB),
    ("fldlg2", 0xD9, 0xEC),
    ("fldln2", 0xD9, 0xED),
    ("fldz", 0xD9, 0xEE),
    ("fchs", 0xD9, 0xE0),
    ("fabs", 0xD9, 0xE1),
    ("ftst", 0xD9, 0xE4),
    ("fxam", 0xD9, 0xE5),
    ("f2xm1", 0xD9, 0xF0),
    ("fyl2x", 0xD9, 0xF1),
    ("fptan", 0xD9, 0xF2),
    ("fpatan", 0xD9, 0xF3),
    ("fxtract", 0xD9, 0xF4),
    ("fprem1", 0xD9, 0xF5),
    ("fdecstp", 0xD9, 0xF6),
    ("fincstp", 0xD9, 0xF7),
    ("fprem", 0xD9, 0xF8),
    ("fyl2xp1", 0xD9, 0xF9),
    ("fsqrt", 0xD9, 0xFA),
    ("fsincos", 0xD9, 0xFB),
    ("frndint", 0xD9, 0xFC),
    ("fscale", 0xD9, 0xFD),
    ("fsin", 0xD9, 0xFE),
    ("fcos", 0xD9, 0xFF),
    ("fnop", 0xD9, 0xD0),
    ("fnstsw", 0xDF, 0xE0),
    ("fnclex", 0xDB, 0xE2),
    ("fninit", 0xDB, 0xE3),
    ("fcompp", 0xDE, 0xD9),
    ("fucompp", 0xDA, 0xE9),
    ("fcom", 0xD8, 0xD1),
    ("fcomp", 0xD8, 0xD9),
    // written without an operand these mean st(1), which is the whole point
    // of them: fold the top of the stack into the one below and pop
    ("faddp", 0xDE, 0xC1),
    ("fmulp", 0xDE, 0xC9),
    ("fsubp", 0xDE, 0xE9),
    ("fsubrp", 0xDE, 0xE1),
    ("fdivp", 0xDE, 0xF9),
    ("fdivrp", 0xDE, 0xF1),
    ("fucom", 0xDD, 0xE1),
    ("fucomp", 0xDD, 0xE9),
];

fn x87(emitter: &mut Emitter, name: &str, ops: &[Operand]) -> Result<()> {
    if ops.is_empty() {
        if let Some((_, a, b)) = X87_BARE.iter().find(|(m, ..)| *m == name) {
            return two(emitter, *a, *b);
        }
    }
    match name {
        "fwait" | "wait" => {
            emitter.byte(0x9B);
            return Ok(());
        }
        // the waiting forms are the same instruction behind a wait
        "fstsw" | "fstcw" | "fclex" | "finit" | "fstenv" | "fsave" => {
            emitter.byte(0x9B);
            return x87(emitter, &format!("fn{}", &name[1..]), ops);
        }
        // the one place the status word goes somewhere other than memory
        "fnstsw" if ops.first().and_then(Operand::as_reg).map(|r| r.is_gpr()) == Some(true) => {
            return two(emitter, 0xDF, 0xE0)
        }
        _ => {}
    }

    let stack = |op: Option<&Operand>| {
        op.and_then(Operand::as_reg)
            .filter(|r| r.class == Class::St)
    };

    /* `fadd st(1),st` and `fadd st,st(1)` are different instructions: the
    first leaves the answer down the stack, the second at the top. and the
    subtract and divide pairs swap meaning between the two, which is a real
    quirk of the encoding rather than a mistake here. */
    if let Some(reg) = stack(ops.first()).filter(|r| r.num != 0 && ops.len() > 1) {
        let base = match name {
            "fadd" => 0xC0,
            "fmul" => 0xC8,
            "fsubr" => 0xE0,
            "fsub" => 0xE8,
            "fdivr" => 0xF0,
            "fdiv" => 0xF8,
            _ => 0,
        };
        if base != 0 {
            return two(emitter, 0xDC, base + reg.num);
        }
    }

    // `fadd st(0),st(1)` names both ends. only one of them is ever the stack
    // top, so the other one is what the opcode encodes
    let picked = stack(ops.first())
        .filter(|r| r.num != 0)
        .or_else(|| stack(ops.get(1)))
        .or_else(|| ops.first().and_then(Operand::as_reg));

    if let Some(reg) = picked.filter(|r| r.class == Class::St) {
        let (opcode, base) = match name {
            "fld" => (0xD9, 0xC0),
            "fst" => (0xDD, 0xD0),
            "fstp" => (0xDD, 0xD8),
            "fadd" => (0xD8, 0xC0),
            "faddp" => (0xDE, 0xC0),
            "fmul" => (0xD8, 0xC8),
            "fmulp" => (0xDE, 0xC8),
            "fsub" => (0xD8, 0xE0),
            "fsubp" => (0xDE, 0xE8),
            "fsubr" => (0xD8, 0xE8),
            "fsubrp" => (0xDE, 0xE0),
            "fdiv" => (0xD8, 0xF0),
            "fdivp" => (0xDE, 0xF8),
            "fdivr" => (0xD8, 0xF8),
            "fdivrp" => (0xDE, 0xF0),
            "fcom" => (0xD8, 0xD0),
            "fcomp" => (0xD8, 0xD8),
            "fcomi" => (0xDB, 0xF0),
            "fcomip" => (0xDF, 0xF0),
            "fucomi" => (0xDB, 0xE8),
            "fucomip" => (0xDF, 0xE8),
            "fucom" => (0xDD, 0xE0),
            "fucomp" => (0xDD, 0xE8),
            "fxch" => (0xD9, 0xC8),
            "ffree" => (0xDD, 0xC0),
            _ => return Err(AsmError::UnknownMnemonic(name.to_string())),
        };
        return two(emitter, opcode, base + reg.num);
    }

    let target = ops.first().ok_or_else(|| bad(name))?;
    let rm = to_rm(target)?;
    let size = rm.size_hint().unwrap_or(4);

    // the integer ones read the operand size off the mnemonic's neighbours:
    // dword is DA, word is DE, and the group of eight is in the same order
    if let Some(rest) = name.strip_prefix("fi") {
        let digit = match rest {
            "add" => 0,
            "mul" => 1,
            "com" => 2,
            "comp" => 3,
            "sub" => 4,
            "subr" => 5,
            "div" => 6,
            "divr" => 7,
            _ => 8,
        };
        if digit < 8 {
            return emitter.encode_x87(if size == 2 { 0xDE } else { 0xDA }, digit, &rm);
        }
    }

    let (opcode, digit) = match (name, size) {
        ("fld", 4) => (0xD9, 0),
        ("fld", 8) => (0xDD, 0),
        ("fld", 10) => (0xDB, 5),
        ("fst", 4) => (0xD9, 2),
        ("fst", 8) => (0xDD, 2),
        ("fstp", 4) => (0xD9, 3),
        ("fstp", 8) => (0xDD, 3),
        ("fstp", 10) => (0xDB, 7),
        ("fild", 2) => (0xDF, 0),
        ("fild", 4) => (0xDB, 0),
        ("fild", 8) => (0xDF, 5),
        ("fist", 2) => (0xDF, 2),
        ("fist", 4) => (0xDB, 2),
        ("fistp", 2) => (0xDF, 3),
        ("fistp", 4) => (0xDB, 3),
        ("fistp", 8) => (0xDF, 7),
        ("fadd", 4) => (0xD8, 0),
        ("fadd", 8) => (0xDC, 0),
        ("fmul", 4) => (0xD8, 1),
        ("fmul", 8) => (0xDC, 1),
        ("fcom", 4) => (0xD8, 2),
        ("fcom", 8) => (0xDC, 2),
        ("fcomp", 4) => (0xD8, 3),
        ("fcomp", 8) => (0xDC, 3),
        ("fsub", 4) => (0xD8, 4),
        ("fsub", 8) => (0xDC, 4),
        ("fsubr", 4) => (0xD8, 5),
        ("fsubr", 8) => (0xDC, 5),
        ("fdiv", 4) => (0xD8, 6),
        ("fdiv", 8) => (0xDC, 6),
        ("fdivr", 4) => (0xD8, 7),
        ("fdivr", 8) => (0xDC, 7),
        ("fldcw", _) => (0xD9, 5),
        ("fnstcw", _) => (0xD9, 7),
        ("fnstsw", _) => (0xDD, 7),
        ("fldenv", _) => (0xD9, 4),
        ("fnstenv", _) => (0xD9, 6),
        ("frstor", _) => (0xDD, 4),
        ("fnsave", _) => (0xDD, 6),
        ("fbld", _) => (0xDF, 4),
        ("fbstp", _) => (0xDF, 6),
        _ => return Err(AsmError::UnknownMnemonic(name.to_string())),
    };

    emitter.encode_x87(opcode, digit, &rm)
}

fn two(emitter: &mut Emitter, a: u8, b: u8) -> Result<()> {
    emitter.byte(a);
    emitter.byte(b);
    Ok(())
}

const SSE_MOVES: &[(&str, Option<u8>, u8, u8)] = &[
    ("movss", Some(0xF3), 0x10, 0x11),
    ("movsd", Some(0xF2), 0x10, 0x11),
    ("movups", None, 0x10, 0x11),
    ("movupd", Some(0x66), 0x10, 0x11),
    ("movaps", None, 0x28, 0x29),
    ("movapd", Some(0x66), 0x28, 0x29),
    ("movdqa", Some(0x66), 0x6F, 0x7F),
    ("movdqu", Some(0xF3), 0x6F, 0x7F),
    ("movlps", None, 0x12, 0x13),
    ("movhps", None, 0x16, 0x17),
    ("movlpd", Some(0x66), 0x12, 0x13),
    ("movhpd", Some(0x66), 0x16, 0x17),
];

fn sse(emitter: &mut Emitter, name: &str, ops: &[Operand]) -> Result<()> {
    if let Some(plain) = name.strip_prefix('v') {
        if known_sse(plain) {
            return avx(emitter, plain, ops);
        }
    }

    let dst = ops
        .first()
        .ok_or_else(|| AsmError::UnknownMnemonic(name.to_string()))?;
    let src = ops
        .get(1)
        .ok_or_else(|| AsmError::UnknownMnemonic(name.to_string()))?;

    if let Some((_, prefix, opcode)) = SSE_ARITH.iter().find(|(m, _, _)| *m == name) {
        let reg = dst.as_reg().ok_or_else(|| bad(name))?;
        let rm = to_rm(src)?;
        return emitter.encode_sse(*prefix, &[0x0F, *opcode], reg, &rm, false);
    }

    if let Some((_, prefix, opcode)) = SSE_IMM8.iter().find(|(m, _, _)| *m == name) {
        let reg = dst.as_reg().ok_or_else(|| bad(name))?;
        let rm = to_rm(src)?;
        let Some(Operand::Imm(pick)) = ops.get(2) else {
            return Err(bad(name));
        };
        emitter.encode_sse(*prefix, &[0x0F, *opcode], reg, &rm, false)?;
        emitter.byte(*pick as u8);
        return Ok(());
    }

    // `cmpeqps xmm0,xmm1` is `cmpps xmm0,xmm1,0` with the predicate spelled out
    if let Some((rest, which)) = name.strip_prefix("cmp").and_then(spelt_out) {
        let real = format!("cmp{rest}");
        if let Some((_, prefix, opcode)) = SSE_IMM8.iter().find(|(m, ..)| **m == real) {
            let reg = dst.as_reg().ok_or_else(|| bad(name))?;
            let rm = to_rm(src)?;
            emitter.encode_sse(*prefix, &[0x0F, *opcode], reg, &rm, false)?;
            emitter.byte(which);
            return Ok(());
        }
    }

    if let Some((_, escape, opcode)) = SSE_38.iter().find(|(m, _, _)| *m == name) {
        let reg = dst.as_reg().ok_or_else(|| bad(name))?;
        let rm = to_rm(src)?;
        let Some(Operand::Imm(pick)) = ops.get(2) else {
            return Err(bad(name));
        };
        emitter.encode_sse(Some(0x66), &[0x0F, *escape, *opcode], reg, &rm, false)?;
        emitter.byte(*pick as u8);
        return Ok(());
    }

    if let Some((_, prefix, load, store)) = SSE_MOVES.iter().find(|(m, _, _, _)| *m == name) {
        if let Some(reg) = dst
            .as_reg()
            .filter(|r| matches!(r.class, Class::Xmm | Class::Ymm))
        {
            let rm = to_rm(src)?;
            return emitter.encode_sse(*prefix, &[0x0F, *load], reg, &rm, false);
        }
        let reg = src.as_reg().ok_or_else(|| bad(name))?;
        let rm = to_rm(dst)?;
        return emitter.encode_sse(*prefix, &[0x0F, *store], reg, &rm, false);
    }

    // the shifts put the count in an immediate and the opcode digit where the
    // register field would be
    if let Some((opcode, digit)) = match name {
        "psrlw" => Some((0x71, 2)),
        "psraw" => Some((0x71, 4)),
        "psllw" => Some((0x71, 6)),
        "psrld" => Some((0x72, 2)),
        "psrad" => Some((0x72, 4)),
        "pslld" => Some((0x72, 6)),
        "psrlq" => Some((0x73, 2)),
        "psrldq" => Some((0x73, 3)),
        "psllq" => Some((0x73, 6)),
        "pslldq" => Some((0x73, 7)),
        _ => None,
    } {
        let reg = dst.as_reg().ok_or_else(|| bad(name))?;
        let Operand::Imm(count) = src else {
            return Err(bad(name));
        };
        let field = operand::Reg {
            class: Class::Xmm,
            num: digit,
        };
        emitter.byte(0x66);
        emitter.encode_sse(None, &[0x0F, opcode], field, &Rm::Reg(reg), false)?;
        emitter.byte(*count as u8);
        return Ok(());
    }

    match name {
        "movmskps" | "movmskpd" | "pmovmskb" => {
            let reg = dst.as_reg().ok_or_else(|| bad(name))?;
            let rm = to_rm(src)?;
            let (prefix, opcode) = match name {
                "movmskps" => (None, 0x50),
                "movmskpd" => (Some(0x66), 0x50),
                _ => (Some(0x66), 0xD7),
            };
            emitter.encode_sse(prefix, &[0x0F, opcode], reg, &rm, false)
        }
        "ptest" => {
            let reg = dst.as_reg().ok_or_else(|| bad(name))?;
            let rm = to_rm(src)?;
            emitter.encode_sse(Some(0x66), &[0x0F, 0x38, 0x17], reg, &rm, false)
        }
        "cvtsi2ss" | "cvtsi2sd" => {
            let reg = dst.as_reg().ok_or_else(|| bad(name))?;
            let rm = to_rm(src)?;
            let prefix = if name.ends_with("ss") { 0xF3 } else { 0xF2 };
            let wide = rm.size_hint() == Some(8);
            emitter.encode_sse(Some(prefix), &[0x0F, 0x2A], reg, &rm, wide)
        }
        "cvttss2si" | "cvttsd2si" | "cvtss2si" | "cvtsd2si" => {
            let reg = dst.as_reg().ok_or_else(|| bad(name))?;
            let rm = to_rm(src)?;
            let prefix = if name.contains("ss") { 0xF3 } else { 0xF2 };
            let opcode = if name.starts_with("cvtt") { 0x2C } else { 0x2D };
            emitter.encode_sse(Some(prefix), &[0x0F, opcode], reg, &rm, reg.size() == 8)
        }
        "movd" | "movq" => {
            let wide = name == "movq";
            if let Some(reg) = dst
                .as_reg()
                .filter(|r| matches!(r.class, Class::Xmm | Class::Ymm))
            {
                let rm = to_rm(src)?;
                emitter.encode_sse(Some(0x66), &[0x0F, 0x6E], reg, &rm, wide)
            } else {
                let reg = src.as_reg().ok_or_else(|| bad(name))?;
                let rm = to_rm(dst)?;
                emitter.encode_sse(Some(0x66), &[0x0F, 0x7E], reg, &rm, wide)
            }
        }
        _ => Err(AsmError::UnknownMnemonic(name.to_string())),
    }
}

fn spelt_out(name: &str) -> Option<(&str, u8)> {
    for (n, word) in ["eq", "lt", "le", "unord", "neq", "nlt", "nle", "ord"]
        .iter()
        .enumerate()
    {
        if let Some(rest) = name.strip_prefix(word) {
            if matches!(rest, "ps" | "pd" | "ss" | "sd") {
                return Some((rest, n as u8));
            }
        }
    }
    None
}

fn known_sse(name: &str) -> bool {
    SSE_ARITH.iter().any(|(m, ..)| *m == name)
        || SSE_IMM8.iter().any(|(m, ..)| *m == name)
        || SSE_MOVES.iter().any(|(m, ..)| *m == name)
        || matches!(
            name,
            "cvtsi2ss"
                | "cvtsi2sd"
                | "cvttss2si"
                | "cvttsd2si"
                | "cvtss2si"
                | "cvtsd2si"
                | "movd"
                | "movq"
        )
}

/* `vaddss xmm0,xmm1,xmm2` is `addss` with a vex prefix and somewhere to put
the second source. written with two operands it means the same as the sse
form, so the destination doubles as the source that vex wants.

a move or a compare has no second source at all, and vvvv has to stay unset
or the cpu reads it as one. the exception is the register form of vmovss and
vmovsd, which really is a three operand merge. */
fn avx(emitter: &mut Emitter, plain: &str, ops: &[Operand]) -> Result<()> {
    let name = format!("v{plain}");
    let dst = ops.first().ok_or_else(|| bad(&name))?;
    let src = ops.get(1).ok_or_else(|| bad(&name))?;

    let (source, rm_op) = match ops.get(2) {
        Some(third) => (src.as_reg(), third),
        None => (None, src),
    };

    if let Some((_, prefix, opcode)) = SSE_ARITH.iter().find(|(m, ..)| *m == plain) {
        let reg = dst.as_reg().ok_or_else(|| bad(&name))?;
        let rm = to_rm(rm_op)?;
        let vvvv = source.or_else(|| (!compare_only(plain)).then_some(reg));
        return emitter.encode_vex(*prefix, &[0x0F, *opcode], reg, vvvv, &rm, false);
    }

    if let Some((_, prefix, opcode)) = SSE_IMM8.iter().find(|(m, ..)| *m == plain) {
        let Some(Operand::Imm(pick)) = ops.last() else {
            return Err(bad(&name));
        };
        let reg = dst.as_reg().ok_or_else(|| bad(&name))?;
        let rm = to_rm(if ops.len() > 3 { &ops[2] } else { src })?;
        let vvvv = if ops.len() > 3 {
            src.as_reg()
        } else {
            Some(reg)
        };
        emitter.encode_vex(*prefix, &[0x0F, *opcode], reg, vvvv, &rm, false)?;
        emitter.byte(*pick as u8);
        return Ok(());
    }

    if let Some((_, prefix, load, store)) = SSE_MOVES.iter().find(|(m, ..)| *m == plain) {
        let merges = matches!(plain, "movss" | "movsd");
        if let Some(reg) = dst
            .as_reg()
            .filter(|r| matches!(r.class, Class::Xmm | Class::Ymm))
        {
            let rm = to_rm(rm_op)?;
            let vvvv = source.or_else(|| (merges && rm_op.as_reg().is_some()).then_some(reg));
            return emitter.encode_vex(*prefix, &[0x0F, *load], reg, vvvv, &rm, false);
        }
        let reg = src.as_reg().ok_or_else(|| bad(&name))?;
        let rm = to_rm(dst)?;
        return emitter.encode_vex(*prefix, &[0x0F, *store], reg, None, &rm, false);
    }

    match plain {
        "cvtsi2ss" | "cvtsi2sd" => {
            let reg = dst.as_reg().ok_or_else(|| bad(&name))?;
            let rm = to_rm(rm_op)?;
            let prefix = if plain.ends_with("ss") { 0xF3 } else { 0xF2 };
            let wide = rm.size_hint() == Some(8);
            emitter.encode_vex(
                Some(prefix),
                &[0x0F, 0x2A],
                reg,
                source.or(Some(reg)),
                &rm,
                wide,
            )
        }
        "cvttss2si" | "cvttsd2si" | "cvtss2si" | "cvtsd2si" => {
            let reg = dst.as_reg().ok_or_else(|| bad(&name))?;
            let rm = to_rm(rm_op)?;
            let prefix = if plain.contains("ss") { 0xF3 } else { 0xF2 };
            let opcode = if plain.starts_with("cvtt") {
                0x2C
            } else {
                0x2D
            };
            emitter.encode_vex(
                Some(prefix),
                &[0x0F, opcode],
                reg,
                None,
                &rm,
                reg.size() == 8,
            )
        }
        "movd" | "movq" => {
            let wide = plain == "movq";
            if let Some(reg) = dst
                .as_reg()
                .filter(|r| matches!(r.class, Class::Xmm | Class::Ymm))
            {
                let rm = to_rm(rm_op)?;
                emitter.encode_vex(Some(0x66), &[0x0F, 0x6E], reg, None, &rm, wide)
            } else {
                let reg = src.as_reg().ok_or_else(|| bad(&name))?;
                let rm = to_rm(dst)?;
                emitter.encode_vex(Some(0x66), &[0x0F, 0x7E], reg, None, &rm, wide)
            }
        }
        _ => Err(AsmError::UnknownMnemonic(name)),
    }
}

// nothing to merge into, so vvvv stays unset. the scalar converts are not in
// here: they keep the top of the destination and need somewhere to take it from
fn compare_only(name: &str) -> bool {
    matches!(
        name,
        "comiss"
            | "comisd"
            | "ucomiss"
            | "ucomisd"
            | "sqrtps"
            | "sqrtpd"
            | "cvtdq2ps"
            | "cvtps2pd"
            | "cvtpd2ps"
            | "cvtps2dq"
            | "cvttps2dq"
            | "cvtdq2pd"
            | "cvtpd2dq"
            | "cvttpd2dq"
    )
}

pub fn nops(count: usize) -> Vec<u8> {
    vec![0x90; count]
}

pub fn jump_patch(from: u64, to: u64, filler: usize) -> Result<Vec<u8>> {
    let delta = to as i64 - (from as i64 + 5);
    if !(-(1i64 << 31)..(1i64 << 31)).contains(&delta) {
        return Err(AsmError::OutOfRange { from, target: to });
    }
    let mut out = Vec::with_capacity(5 + filler);
    out.push(0xE9);
    out.extend_from_slice(&(delta as i32).to_le_bytes());
    out.extend(std::iter::repeat_n(0x90u8, filler));
    Ok(out)
}
