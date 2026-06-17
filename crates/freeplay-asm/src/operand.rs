use crate::error::{AsmError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Gpr8,
    Gpr8Rex,
    Gpr16,
    Gpr32,
    Gpr64,
    Xmm,
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
        let (digits, class) = match rest.strip_suffix('b') {
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

    if let Some(digits) = lower.strip_prefix("xmm") {
        if let Ok(num) = digits.parse::<u8>() {
            if num <= 15 {
                return Some(Reg {
                    class: Class::Xmm,
                    num,
                });
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
        "dword" | "real4" => Some(4),
        "qword" | "real8" => Some(8),
        "tword" | "tbyte" | "real10" => Some(10),
        "xmmword" | "oword" => Some(16),
        _ => None,
    }
}

pub fn number(text: &str) -> Result<i64> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err(AsmError::Operand(text.to_string()));
    }

    let (negative, body) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest.trim()),
        None => (false, raw.strip_prefix('+').unwrap_or(raw).trim()),
    };

    let value = if let Some(dec) = body.strip_prefix('#') {
        dec.parse::<i64>()
            .map_err(|_| AsmError::Operand(text.to_string()))?
    } else if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).map_err(|_| AsmError::Operand(text.to_string()))?
    } else {
        let hex = body.strip_suffix('h').unwrap_or(body);
        i64::from_str_radix(hex, 16).map_err(|_| AsmError::Operand(text.to_string()))?
    };

    Ok(if negative { -value } else { value })
}

pub fn is_number(text: &str) -> bool {
    number(text).is_ok()
}

pub fn is_identifier(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && t.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

pub fn parse_operand(text: &str) -> Result<Operand> {
    let mut body = text.trim();
    let mut size = None;

    loop {
        let lower = body.to_ascii_lowercase();
        let mut advanced = false;
        for word in [
            "byte", "word", "dword", "qword", "tword", "tbyte", "xmmword", "oword", "real4",
            "real8", "real10",
        ] {
            if let Some(rest) = lower.strip_prefix(word) {
                if rest.starts_with(char::is_whitespace) || rest.starts_with('[') {
                    size = size_keyword(word);
                    body = body[word.len()..].trim_start();
                    advanced = true;
                    break;
                }
            }
        }
        if !advanced {
            break;
        }
        if let Some(rest) = body.to_ascii_lowercase().strip_prefix("ptr") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('[') {
                body = body[3..].trim_start();
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
        return Ok(Operand::Mem(mem));
    }

    if let Some(reg) = register(body) {
        return Ok(Operand::Reg(reg));
    }

    if is_number(body) {
        return Ok(Operand::Imm(number(body)?));
    }

    if is_identifier(body) {
        return Ok(Operand::Symbol(body.to_string()));
    }

    Err(AsmError::Operand(text.to_string()))
}

fn parse_memory(inner: &str) -> Result<Mem> {
    let mut mem = Mem::default();
    let mut term = String::new();
    let mut negative = false;
    let mut terms: Vec<(bool, String)> = Vec::new();

    for ch in inner.chars() {
        match ch {
            '+' => {
                terms.push((negative, term.trim().to_string()));
                term.clear();
                negative = false;
            }
            '-' => {
                terms.push((negative, term.trim().to_string()));
                term.clear();
                negative = true;
            }
            other => term.push(other),
        }
    }
    terms.push((negative, term.trim().to_string()));

    for (neg, piece) in terms {
        if piece.is_empty() {
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

        if is_identifier(&piece) {
            if mem.symbol.is_some() {
                return Err(AsmError::Operand(inner.to_string()));
            }
            mem.symbol = Some(piece);
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

    for ch in text.chars() {
        match ch {
            '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            ']' | ')' => {
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
}
