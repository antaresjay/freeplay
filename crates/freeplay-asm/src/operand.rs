use crate::error::{AsmError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Gpr8,
    Gpr8Rex,
    Gpr16,
    Gpr32,
    Gpr64,
    Xmm,
    Ymm,
    St,
    Seg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg {
    pub class: Class,
    pub num: u8,
}

impl Reg {
    pub fn size(self) -> usize {
        match self.class {
            Class::Gpr8 | Class::Gpr8Rex => 1,
            Class::Gpr16 => 2,
            Class::Gpr32 => 4,
            Class::Gpr64 => 8,
            Class::Xmm => 16,
            Class::Ymm => 32,
            Class::St => 10,
            Class::Seg => 2,
        }
    }

    pub fn is_gpr(self) -> bool {
        matches!(
            self.class,
            Class::Gpr8 | Class::Gpr8Rex | Class::Gpr16 | Class::Gpr32 | Class::Gpr64
        )
    }

    pub fn extended(self) -> bool {
        self.num >= 8
    }

    pub fn low3(self) -> u8 {
        self.num & 7
    }
}

const GPR8: [&str; 8] = ["al", "cl", "dl", "bl", "ah", "ch", "dh", "bh"];
const GPR8_REX: [&str; 4] = ["spl", "bpl", "sil", "dil"];
const GPR16: [&str; 8] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"];
const GPR32: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];
const GPR64: [&str; 8] = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi"];
const SEG: [&str; 6] = ["es", "cs", "ss", "ds", "fs", "gs"];

pub fn register(name: &str) -> Option<Reg> {
    let lower = name.trim().to_ascii_lowercase();

    if let Some(num) = GPR8.iter().position(|r| *r == lower) {
        return Some(Reg {
            class: Class::Gpr8,
            num: num as u8,
        });
    }
    if let Some(num) = GPR8_REX.iter().position(|r| *r == lower) {
        return Some(Reg {
            class: Class::Gpr8Rex,
            num: num as u8 + 4,
        });
    }
    if let Some(num) = GPR16.iter().position(|r| *r == lower) {
        return Some(Reg {
            class: Class::Gpr16,
            num: num as u8,
        });
    }
    if let Some(num) = GPR32.iter().position(|r| *r == lower) {
        return Some(Reg {
            class: Class::Gpr32,
            num: num as u8,
        });
    }
    if let Some(num) = GPR64.iter().position(|r| *r == lower) {
        return Some(Reg {
            class: Class::Gpr64,
            num: num as u8,
        });
    }
    if let Some(num) = SEG.iter().position(|r| *r == lower) {
        return Some(Reg {
            class: Class::Seg,
            num: num as u8,
        });
    }

    if let Some(rest) = lower.strip_prefix('r') {
        // cheat engine writes r15l where everything else writes r15b
        let (digits, class) = match rest.strip_suffix('b').or(rest.strip_suffix('l')) {
            Some(d) => (d, Class::Gpr8Rex),
            None => match rest.strip_suffix('w') {
                Some(d) => (d, Class::Gpr16),
                None => match rest.strip_suffix('d') {
                    Some(d) => (d, Class::Gpr32),
                    None => (rest, Class::Gpr64),
                },
            },
        };
        if let Ok(num) = digits.parse::<u8>() {
            if (8..=15).contains(&num) {
                return Some(Reg { class, num });
            }
        }
    }

    for (prefix, class) in [("xmm", Class::Xmm), ("ymm", Class::Ymm)] {
        if let Some(digits) = lower.strip_prefix(prefix) {
            if let Ok(num) = digits.parse::<u8>() {
                if num <= 15 {
                    return Some(Reg { class, num });
                }
            }
        }
    }

    if lower == "st" {
        return Some(Reg {
            class: Class::St,
            num: 0,
        });
    }
    if let Some(rest) = lower.strip_prefix("st(") {
        if let Some(digits) = rest.strip_suffix(')') {
            if let Ok(num) = digits.parse::<u8>() {
                if num <= 7 {
                    return Some(Reg {
                        class: Class::St,
                        num,
                    });
                }
            }
        }
    }

    None
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mem {
    pub size: Option<usize>,
    pub base: Option<Reg>,
    pub index: Option<Reg>,
    pub scale: u8,
    pub disp: i64,
    pub symbol: Option<String>,
    pub seg: Option<u8>,
}

impl Mem {
    pub fn is_absolute(&self) -> bool {
        self.base.is_none() && self.index.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Reg(Reg),
    Mem(Mem),
    Imm(i64),
    Symbol(String),
}

impl Operand {
    pub fn as_reg(&self) -> Option<Reg> {
        match self {
            Operand::Reg(r) => Some(*r),
            _ => None,
        }
    }

    pub fn as_mem(&self) -> Option<&Mem> {
        match self {
            Operand::Mem(m) => Some(m),
            _ => None,
        }
    }
}

pub fn size_keyword(word: &str) -> Option<usize> {
    match word.trim().to_ascii_lowercase().as_str() {
        "byte" => Some(1),
        "word" => Some(2),
        "dword" | "real4" | "float" | "int" | "long" => Some(4),
        "qword" | "real8" | "double" => Some(8),
        "tword" | "tbyte" | "real10" => Some(10),
        "xmmword" | "oword" | "dqword" => Some(16),
        _ => None,
    }
}

const SIZE_WORDS: &[&str] = &[
    "byte", "word", "dword", "qword", "tword", "tbyte", "xmmword", "oword", "dqword", "real4",
    "real8", "real10",
];

// cheat engine writes the bit pattern it wants rather than making you work it
// out: `dd (float)1.25` and `mov edx,(float)1000.0`. the cast decides how the
// digits are read, and everything after it is decimal, not the hex this
// otherwise assumes
fn cast(text: &str) -> Option<Result<i64>> {
    let raw = text.trim();
    let close = raw.find(')')?;
    let kind = raw.strip_prefix('(')?[..close - 1]
        .trim()
        .to_ascii_lowercase();
    let digits = raw[close + 1..].trim();

    let bad = || AsmError::Operand(text.to_string());
    Some(match kind.as_str() {
        "float" => special(digits)
            .map(|f| f as f32)
            .ok_or(())
            .or_else(|_| digits.parse::<f32>().map_err(|_| ()))
            .map(|f| f.to_bits() as i64)
            .map_err(|_| bad()),
        "double" => special(digits)
            .ok_or(())
            .or_else(|_| digits.parse::<f64>().map_err(|_| ()))
            .map(|f| f.to_bits() as i64)
            .map_err(|_| bad()),
        "int" | "dword" | "long" | "byte" | "word" | "qword" | "char" => whole(digits, bad),
        _ => return None,
    })
}

// decimal is what the cast is for, but a table that writes `(dword)C` or
// `(int)0.5` still meant something
fn whole(digits: &str, bad: impl Fn() -> AsmError) -> Result<i64> {
    if let Ok(value) = digits.parse::<i64>() {
        return Ok(value);
    }
    if let Ok(value) = digits.parse::<f64>() {
        return Ok(value as i64);
    }
    i64::from_str_radix(digits, 16).map_err(|_| bad())
}

fn special(digits: &str) -> Option<f64> {
    match digits.trim().to_ascii_lowercase().as_str() {
        "inf" | "+inf" | "infinity" => Some(f64::INFINITY),
        "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
        "nan" | "+nan" | "-nan" => Some(f64::NAN),
        _ => None,
    }
}

pub fn number(text: &str) -> Result<i64> {
    let mut raw = text.trim();
    if raw.is_empty() {
        return Err(AsmError::Operand(text.to_string()));
    }

    // a closing bracket with nothing that opened it, left over from an edit
    while raw.ends_with(')') && raw.matches('(').count() < raw.matches(')').count() {
        raw = raw[..raw.len() - 1].trim_end();
    }

    if let Some(found) = cast(raw) {
        return found;
    }

    // 'Bott' is four bytes, not a string. tables use it to compare against a
    // tag the game keeps inline, and it packs little endian like any number.
    // anything after the closing quote carries on above the characters
    if let Some(rest) = raw.strip_prefix('\'') {
        if let Some(close) = rest.find('\'') {
            let bytes = &rest.as_bytes()[..close];
            if bytes.is_empty() || bytes.len() > 8 {
                return Err(AsmError::Operand(text.to_string()));
            }
            let mut value = 0u64;
            for (n, byte) in bytes.iter().enumerate() {
                value |= (*byte as u64) << (8 * n);
            }
            let tail = rest[close + 1..].trim();
            if !tail.is_empty() {
                let above = number(tail)? as u64;
                value |= above << (8 * bytes.len());
            }
            return Ok(value as i64);
        }
    }

    // `+Inf` and `1.0` written straight out. no cast in front of it, so the
    // width is the one everything in a game is: four bytes
    if let Some(value) = special(raw) {
        return Ok((value as f32).to_bits() as i64);
    }
    if raw.contains('.') && !raw.contains(['[', ']']) {
        if let Ok(value) = raw.parse::<f32>() {
            return Ok(value.to_bits() as i64);
        }
    }

    // `mov [r15+79*4],1` and `mov [esi+58],100-6`, worked out here rather than
    // by the person writing it
    for sign in ['*', '+', '-'] {
        let Some(at) = raw[1..].rfind(sign).map(|at| at + 1) else {
            continue;
        };
        let (left, right) = (&raw[..at], &raw[at + 1..]);
        if let (Ok(a), Ok(b)) = (number(left), number(right)) {
            return Ok(match sign {
                '*' => a.wrapping_mul(b),
                '+' => a.wrapping_add(b),
                _ => a.wrapping_sub(b),
            });
        }
    }

    let (negative, body) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest.trim()),
        None => (false, raw.strip_prefix('+').unwrap_or(raw).trim()),
    };

    let value = if let Some(dec) = body.strip_prefix('#') {
        dec.parse::<i64>()
            .map_err(|_| AsmError::Operand(text.to_string()))?
    // `$10` is hex, the pascal spelling cheat engine inherited from delphi
    } else if let Some(hex) = body.strip_prefix('$').filter(|h| !h.is_empty()) {
        // wide enough for an address written out in full, which overflows i64
        u64::from_str_radix(hex, 16).map_err(|_| AsmError::Operand(text.to_string()))? as i64
    } else if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        // wide enough for an address written out in full, which overflows i64
        u64::from_str_radix(hex, 16).map_err(|_| AsmError::Operand(text.to_string()))? as i64
    } else {
        let hex = body.strip_suffix('h').unwrap_or(body);
        // wide enough for an address written out in full, which overflows i64.
        // too wide even for that and it was decimal all along, whatever the
        // usual rule says
        match u64::from_str_radix(hex, 16) {
            Ok(value) => value as i64,
            Err(_) => body
                .parse::<i64>()
                .map_err(|_| AsmError::Operand(text.to_string()))?,
        }
    };

    Ok(if negative { -value } else { value })
}

pub fn is_number(text: &str) -> bool {
    number(text).is_ok()
}

// `$process` is cheat engine's name for the main module. the rest of the odd
// characters come from symbols lifted out of a c++ binary, which arrive with
// their namespaces, template arguments and operator names attached
const IN_A_NAME: &str = "_.:$<>~=?!@-`|&";

/* a dash is in there because module names have them, and the arithmetic still
works: a symbol is looked up whole first and only split on the last sign if
that misses.

a leading digit is allowed too, for `20XX.exe`. everything that calls this
tries a number first, so nothing that is really a number gets here, but it
still has to have a letter in it somewhere or `100+200` would come through as
a name. */
pub fn is_identifier(text: &str) -> bool {
    let t = text.trim();
    t.chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || "_$@.".contains(c) || !c.is_ascii())
        && t.chars().any(|c| c.is_alphabetic() || c == '_' || c == '@')
        && t.chars()
            .all(|c| c.is_alphanumeric() || IN_A_NAME.contains(c) || !c.is_ascii())
}

// `"Game Name.exe"+1234`. the quotes are only there so the name can have a
// space in it and mean nothing once it is a symbol
pub fn unquote(text: &str) -> Option<String> {
    let raw = text.trim();
    let rest = raw.strip_prefix('"').or_else(|| raw.strip_prefix("$\""))?;
    let close = rest.find('"')?;
    let name = rest[..close].trim();
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || " ,'&()[]".contains(c) || IN_A_NAME.contains(c));
    if !ok {
        return None;
    }
    // a space is only allowed while the quotes are on, so whatever follows
    // the closing one has to be an ordinary continuation of the name
    let tail = rest[close + 1..].trim();
    if !tail.is_empty() && !tail.starts_with(['.', '+', '-']) {
        return None;
    }
    Some(format!("{name}{tail}"))
}

// `FalloutNV.exe+39040D` splits into the name and what to add to it. the
// number is on the right of the last sign, and only if it really is one:
// a symbol is allowed to have a dash in it
pub fn split_offset(name: &str) -> (&str, Option<&str>) {
    let text = name.trim();
    match text.rfind(['+', '-']) {
        Some(at) if at > 0 && number(&text[at..]).is_ok() => (text[..at].trim(), Some(&text[at..])),
        _ => (text, None),
    }
}

// a symbol with arithmetic hung off it, which is how cheat engine writes most
// addresses. resolved when the fixups are applied, since neither half means
// anything until the module is loaded
// `adreslist+changedboleanlist`, two names added together, which neither
// is_identifier nor an offset covers. every piece has to stand on its own and
// at least one of them has to be a name
pub fn is_sum(text: &str) -> bool {
    let mut names = 0;
    for piece in text.split(['+', '-']).map(str::trim) {
        if register(piece).is_some() {
            return false;
        }
        if is_identifier(piece) {
            names += 1;
        } else if !is_number(piece) {
            return false;
        }
    }
    names > 0 && text.contains(['+', '-'])
}

// what is left of `edi+7*8+30` once every offset has come off it
pub fn root(text: &str) -> &str {
    match split_offset(text) {
        (head, Some(_)) if head.len() < text.trim().len() => root(head),
        (head, _) => head,
    }
}

pub fn is_identifier_with_offset(text: &str) -> bool {
    let (head, extra) = split_offset(text);
    // more than one, as in `AmmoPatch+2+4`
    extra.is_some() && (is_identifier(head) || is_identifier_with_offset(head))
}

pub fn parse_operand(text: &str) -> Result<Operand> {
    let mut body = text.trim();
    let mut size = None;

    loop {
        let lower = body.to_ascii_lowercase();
        let mut advanced = false;
        for word in SIZE_WORDS {
            if let Some(rest) = lower.strip_prefix(word) {
                if rest.starts_with(char::is_whitespace) || rest.starts_with('[') {
                    size = size_keyword(word);
                    body = body[word.len()..].trim_start();
                    advanced = true;
                    break;
                }
            }
        }
        // `mov eax,(float)[setHP]` and `movss xmm0,(float)[t]`. next to a
        // bracket the cast says how wide the read is, not what to encode, and
        // it lands on either side of it depending on who wrote the table
        if !advanced {
            if let Some((width, rest)) = bracket_cast(body) {
                size = Some(width);
                body = rest;
                advanced = true;
            } else if let Some((width, rest)) = trailing_cast(body) {
                size = Some(width);
                body = rest;
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
        // `prt` turns up often enough to be worth taking as the same word
        let lower = body.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("ptr").or(lower.strip_prefix("prt")) {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('[') {
                body = body[3..].trim_start();
            }
        }
    }

    // `cmp rdi+B0],2` with no opening bracket, and `mov [rax+18]],1` with one
    // closing bracket too many. both are edits nobody finished
    let patched;
    let opens = body.matches('[').count();
    let shuts = body.matches(']').count();
    if body.ends_with(']') && opens < shuts {
        patched = if opens == 0 {
            format!("[{body}")
        } else {
            body[..body.len() - 1].to_string()
        };
        body = &patched;
    }
    // `add ebx,#[fcraft]`, where the decimal marker landed on a memory read
    if let Some(rest) = body.strip_prefix('#').filter(|r| r.starts_with('[')) {
        body = rest;
    }

    // `fs:[30]`, which is how a script reaches the thread block
    let mut seg = None;
    if let Some(at) = body.find(':') {
        if let Some(reg) = register(&body[..at]).filter(|r| r.class == Class::Seg) {
            let rest = body[at + 1..].trim_start();
            if rest.starts_with('[') {
                seg = Some(reg.num);
                body = rest;
            }
        }
    }

    if body.starts_with('[') {
        let inner = body
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or_else(|| AsmError::Operand(text.to_string()))?;
        let mut mem = parse_memory(inner)?;
        mem.size = size.or(mem.size);
        mem.seg = seg;
        return Ok(Operand::Mem(mem));
    }

    if let Some(reg) = register(body) {
        return Ok(Operand::Reg(reg));
    }

    if is_number(body) {
        return Ok(Operand::Imm(number(body)?));
    }

    if is_identifier(body) || is_identifier_with_offset(body) {
        return Ok(Operand::Symbol(body.to_string()));
    }

    // `jmp CharacterDBSO+Character:get_IsLocked+bf`, where neither half of the
    // sum is a number
    if is_sum(body) && register(root(body)).is_none() {
        return Ok(Operand::Symbol(body.to_string()));
    }

    if let Some(plain) = unquote(body) {
        return Ok(Operand::Symbol(plain));
    }

    Err(AsmError::Operand(text.to_string()))
}

fn bracket_cast(body: &str) -> Option<(usize, &str)> {
    let close = body.strip_prefix('(')?.find(')')? + 1;
    let rest = body[close + 1..].trim_start();
    if !rest.starts_with('[') {
        return None;
    }
    Some((size_keyword(&body[1..close])?, rest))
}

fn trailing_cast(body: &str) -> Option<(usize, &str)> {
    let head = body.strip_suffix(')')?;
    let open = head.rfind('(')?;
    if !head[..open].trim_end().ends_with(']') {
        return None;
    }
    Some((size_keyword(&head[open + 1..])?, head[..open].trim_end()))
}

fn parse_memory(inner: &str) -> Result<Mem> {
    // `[BTD5-Win.exe+209D08]` and `["Game Name.exe"+1234]`. splitting on the
    // signs first would tear the module name in half
    // `[[SanDLL.dll+311F78]+C0]` follows a pointer before it reads anything.
    // there is no encoding for that, so the whole chain stays a symbol and is
    // worked out against the running process
    if inner.trim_start().starts_with('[') {
        return Ok(Mem {
            symbol: Some(inner.trim().to_string()),
            ..Default::default()
        });
    }

    let whole = unquote(inner).or_else(|| {
        let named = register(root(inner)).is_none()
            && (is_identifier(inner) || is_identifier_with_offset(inner) || is_sum(inner));
        named.then(|| inner.trim().to_string())
    });
    if let Some(symbol) = whole {
        return Ok(Mem {
            symbol: Some(symbol),
            ..Default::default()
        });
    }

    let mut mem = Mem::default();
    let mut terms: Vec<(bool, String)> = Vec::new();

    for chunk in inner.split('+') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        // `[CaveStory+.exe+C4054]`, a module whose name really has a plus in it
        if chunk.starts_with('.') {
            if let Some((_, last)) = terms.last_mut() {
                last.push('+');
                last.push_str(chunk);
                continue;
            }
        }
        // a dash is a subtraction after a register or a number, and part of
        // the name after anything else
        let head = chunk.split('-').next().unwrap_or_default();
        let arithmetic = head
            .split('*')
            .all(|p| register(p.trim()).is_some() || is_number(p.trim()));
        if !chunk.contains('-') || !arithmetic {
            terms.push((false, chunk.to_string()));
            continue;
        }
        for (n, bit) in chunk.split('-').enumerate() {
            if !bit.trim().is_empty() {
                terms.push((n > 0, bit.trim().to_string()));
            }
        }
    }

    for (neg, piece) in terms {
        if piece.is_empty() {
            continue;
        }

        // nothing else leaves a plus behind, so this is the glued name above
        if piece.contains('+') {
            mem.symbol = Some(piece);
            continue;
        }

        // `[edi+7*8+30]`, where neither side is a register and the whole term
        // is a number waiting to be worked out
        if is_number(&piece) {
            let value = number(&piece)?;
            mem.disp += if neg { -value } else { value };
            continue;
        }

        if let Some((left, right)) = piece.split_once('*') {
            let (reg_text, scale_text) = match register(left.trim()) {
                Some(_) => (left, right),
                None => (right, left),
            };
            let reg =
                register(reg_text.trim()).ok_or_else(|| AsmError::Operand(piece.to_string()))?;
            let scale = number(scale_text.trim())? as u8;
            if !matches!(scale, 1 | 2 | 4 | 8) {
                return Err(AsmError::Operand(format!("scale {scale}")));
            }
            mem.index = Some(reg);
            mem.scale = scale;
            continue;
        }

        if let Some(reg) = register(&piece) {
            if mem.base.is_none() {
                mem.base = Some(reg);
            } else if mem.index.is_none() {
                mem.index = Some(reg);
                mem.scale = 1;
            } else {
                return Err(AsmError::Operand(inner.to_string()));
            }
            continue;
        }

        if is_number(&piece) {
            let value = number(&piece)?;
            mem.disp += if neg { -value } else { value };
            continue;
        }

        let name = if is_identifier(&piece) {
            Some(piece.clone())
        } else {
            unquote(&piece)
        };
        if let Some(name) = name {
            if mem.symbol.is_some() {
                return Err(AsmError::Operand(inner.to_string()));
            }
            mem.symbol = Some(name);
            continue;
        }

        return Err(AsmError::Operand(piece));
    }

    if mem.index.is_some() && mem.scale == 0 {
        mem.scale = 1;
    }
    Ok(mem)
}

pub fn split_operands(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut quoted = false;

    for ch in text.chars() {
        if quoted {
            current.push(ch);
            quoted = ch != '"';
            continue;
        }
        match ch {
            // a c++ name with template arguments in it is full of commas
            '"' => {
                quoted = true;
                current.push(ch);
            }
            '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            ']' | ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            // `mov [mygg_3],,rax` has one comma too many, and nothing is
            // between them to make an operand out of
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
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

    #[test]
    fn reads_the_register_names() {
        assert_eq!(register("eax").unwrap().num, 0);
        assert_eq!(register("EDI").unwrap().num, 7);
        assert_eq!(register("r13").unwrap().num, 13);
        assert_eq!(register("r13d").unwrap().class, Class::Gpr32);
        assert_eq!(register("xmm7").unwrap().class, Class::Xmm);
        assert_eq!(register("st(3)").unwrap().num, 3);
        assert!(register("notareg").is_none());
    }

    #[test]
    fn numbers_are_hex_unless_told_otherwise() {
        assert_eq!(number("10").unwrap(), 0x10);
        assert_eq!(number("#10").unwrap(), 10);
        assert_eq!(number("0x10").unwrap(), 0x10);
        assert_eq!(number("-8").unwrap(), -8);
        assert_eq!(number("00000234").unwrap(), 0x234);
    }

    #[test]
    fn parses_a_plain_memory_operand() {
        let mem = parse_operand("[ebp+08]").unwrap();
        let mem = mem.as_mem().unwrap();
        assert_eq!(mem.base, register("ebp"));
        assert_eq!(mem.disp, 8);
    }

    #[test]
    fn parses_a_scaled_index() {
        let op = parse_operand("[edi+ecx*4+30]").unwrap();
        let mem = op.as_mem().unwrap();
        assert_eq!(mem.base, register("edi"));
        assert_eq!(mem.index, register("ecx"));
        assert_eq!(mem.scale, 4);
        assert_eq!(mem.disp, 0x30);
    }

    #[test]
    fn two_registers_without_a_star_is_base_plus_index() {
        let op = parse_operand("[edi+ecx+30]").unwrap();
        let mem = op.as_mem().unwrap();
        assert_eq!(mem.base, register("edi"));
        assert_eq!(mem.index, register("ecx"));
        assert_eq!(mem.scale, 1);
    }

    #[test]
    fn a_symbol_can_be_the_whole_address() {
        let op = parse_operand("[baseWitcher]").unwrap();
        let mem = op.as_mem().unwrap();
        assert_eq!(mem.symbol.as_deref(), Some("baseWitcher"));
        assert!(mem.is_absolute());
    }

    #[test]
    fn size_keywords_come_off_the_front() {
        let op = parse_operand("dword ptr [edx+00000234]").unwrap();
        let mem = op.as_mem().unwrap();
        assert_eq!(mem.size, Some(4));
        assert_eq!(mem.disp, 0x234);
    }

    #[test]
    fn negative_displacements_work() {
        let op = parse_operand("[ebp-04]").unwrap();
        assert_eq!(op.as_mem().unwrap().disp, -4);
    }

    #[test]
    fn splits_operands_without_breaking_brackets() {
        let parts = split_operands("eax,[ebx+ecx*2+10]");
        assert_eq!(parts, vec!["eax", "[ebx+ecx*2+10]"]);
    }

    // cheat engine's casts. the digits after one are decimal, unlike a bare
    // number here which is hex, and a float means the bits rather than the
    // value. fallout new vegas will not assemble without these
    #[test]
    fn an_int_cast_is_decimal_not_hex() {
        assert_eq!(number("(int)100").unwrap(), 100);
        assert_eq!(number("(int)500").unwrap(), 500);
        assert_eq!(number("(int)10").unwrap(), 10);
        // without the cast the same digits are hex, which is the whole point
        assert_eq!(number("100").unwrap(), 0x100);
    }

    #[test]
    fn a_float_cast_is_the_bit_pattern() {
        assert_eq!(number("(float)1000.0").unwrap(), 0x447A0000);
        assert_eq!(number("(float)200.0").unwrap(), 0x43480000);
        assert_eq!(number("(float)1.25").unwrap(), 0x3FA00000);
        assert_eq!(number("(float)15.0").unwrap(), 0x41700000);
    }

    #[test]
    fn a_double_cast_is_eight_bytes_of_it() {
        assert_eq!(number("(double)1.0").unwrap(), 0x3FF0000000000000u64 as i64);
    }

    #[test]
    fn spacing_and_case_inside_a_cast_do_not_matter() {
        assert_eq!(number("(FLOAT)1.25").unwrap(), 0x3FA00000);
        assert_eq!(number(" (int) 42 ").unwrap(), 42);
    }

    #[test]
    fn a_cast_of_nonsense_is_an_error_not_a_guess() {
        assert!(number("(float)banana").is_err());
        assert!(number("(int)nothing").is_err());
    }

    // a bracketed thing that is not a cast has to fall through to the normal
    // parse rather than being swallowed
    #[test]
    fn something_that_only_looks_like_a_cast_is_left_alone() {
        assert!(number("(nonsense)10").is_err());
    }
}
