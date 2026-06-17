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
    ("maxss", Some(0xF3), 0x5F),
    ("sqrtss", Some(0xF3), 0x51),
    ("xorps", None, 0x57),
    ("xorpd", Some(0x66), 0x57),
    ("andps", None, 0x54),
    ("andpd", Some(0x66), 0x54),
    ("orps", None, 0x56),
    ("comiss", None, 0x2F),
    ("comisd", Some(0x66), 0x2F),
    ("ucomiss", None, 0x2E),
    ("ucomisd", Some(0x66), 0x2E),
    ("cvtss2sd", Some(0xF3), 0x5A),
    ("cvtsd2ss", Some(0xF2), 0x5A),
    ("cvtdq2ps", None, 0x5B),
    ("cvtps2pd", None, 0x5A),
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
        mut bytes, fixups, ..
    } = emitter;

    for fixup in &fixups {
        let target = labels
            .get(&fixup.symbol)
            .or_else(|| externals.get(&fixup.symbol))
            .copied()
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

    Ok(Assembled { bytes, labels })
}

fn at(line: usize, source: AsmError) -> AsmError {
    AsmError::At {
        line,
        source: Box::new(source),
    }
}

fn read_items(source: &str) -> Result<Vec<Item>> {
    let mut items = Vec::new();

    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let mut text = raw;
        if let Some(at) = text.find("//") {
            text = &text[..at];
        }
        if let Some(at) = text.find(';') {
            text = &text[..at];
        }
        let text = text.trim();
        if text.is_empty() {
            continue;
        }

        let mut rest = text;
        while let Some(colon) = label_colon(rest) {
            let name = rest[..colon].trim().to_string();
            items.push(Item::Label(name));
            rest = rest[colon + 1..].trim();
        }
        if rest.is_empty() {
            continue;
        }

        let (head, tail) = match rest.find(char::is_whitespace) {
            Some(at) => (&rest[..at], rest[at..].trim()),
            None => (rest, ""),
        };
        let head_lower = head.to_ascii_lowercase();

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
            "align" => items.push(Item::Align(
                operand::number(tail).unwrap_or(1).max(1) as usize
            )),
            _ => items.push(Item::Instruction {
                line,
                text: rest.to_string(),
            }),
        }
    }

    Ok(items)
}

fn label_colon(text: &str) -> Option<usize> {
    let at = text.find(':')?;
    let name = text[..at].trim();
    if name.is_empty() || !operand::is_identifier(name) {
        return None;
    }
    if text[at + 1..].starts_with(':') {
        return None;
    }
    Some(at)
}

fn emit_data(emitter: &mut Emitter, width: usize, text: &str) -> Result<()> {
    for piece in split_operands(text)
        .into_iter()
        .flat_map(|p| p.split_whitespace().map(str::to_string).collect::<Vec<_>>())
    {
        let piece = piece.trim();
        if piece.is_empty() {
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
        } else if operand::is_identifier(piece) {
            emitter.fixup(piece, width, 0, false);
        } else {
            return Err(AsmError::Operand(piece.to_string()));
        }
    }
    Ok(())
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
    let (mnemonic, tail) = match text.find(char::is_whitespace) {
        Some(at) => (&text[..at], text[at..].trim()),
        None => (text, ""),
    };
    let name = mnemonic.to_ascii_lowercase();
    let parts = split_operands(tail);
    let ops: Vec<Operand> = parts
        .iter()
        .map(|p| parse_operand(p))
        .collect::<Result<Vec<_>>>()?;

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
        "pushfd" | "pushfq" => {
            emitter.byte(0x9C);
            Ok(())
        }
        "popfd" | "popfq" => {
            emitter.byte(0x9D);
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
        "xchg" => {
            let reg = ops
                .first()
                .and_then(Operand::as_reg)
                .ok_or_else(|| bad(&name))?;
            let rm = to_rm(ops.get(1).ok_or_else(|| bad(&name))?)?;
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
        if a.class == Class::Xmm || b.class == Class::Xmm {
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
        2 => {
            let reg = ops[0].as_reg().ok_or_else(|| bad("imul"))?;
            let rm = to_rm(&ops[1])?;
            emitter.encode_reg_rm(&[0x0F, 0xAF], reg, &rm, reg.size())
        }
        3 => {
            let reg = ops[0].as_reg().ok_or_else(|| bad("imul"))?;
            let rm = to_rm(&ops[1])?;
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

fn x87(emitter: &mut Emitter, name: &str, ops: &[Operand]) -> Result<()> {
    match name {
        "fld1" => return two(emitter, 0xD9, 0xE8),
        "fldz" => return two(emitter, 0xD9, 0xEE),
        "fchs" => return two(emitter, 0xD9, 0xE0),
        "fabs" => return two(emitter, 0xD9, 0xE1),
        "fwait" | "wait" => {
            emitter.byte(0x9B);
            return Ok(());
        }
        "fnstsw" => return two(emitter, 0xDF, 0xE0),
        _ => {}
    }

    if let Some(Operand::Reg(reg)) = ops.first() {
        if reg.class == Class::St {
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
                "fdiv" => (0xD8, 0xF0),
                "fdivp" => (0xDE, 0xF8),
                "fdivr" => (0xD8, 0xF8),
                "fcom" => (0xD8, 0xD0),
                "fcomp" => (0xD8, 0xD8),
                "fxch" => (0xD9, 0xC8),
                "ffree" => (0xDD, 0xC0),
                _ => return Err(AsmError::UnknownMnemonic(name.to_string())),
            };
            return two(emitter, opcode, base + reg.num);
        }
    }

    let target = ops.first().ok_or_else(|| bad(name))?;
    let rm = to_rm(target)?;
    let size = rm.size_hint().unwrap_or(4);

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
        ("fcomp", 4) => (0xD8, 3),
        ("fsub", 4) => (0xD8, 4),
        ("fsub", 8) => (0xDC, 4),
        ("fsubr", 4) => (0xD8, 5),
        ("fdiv", 4) => (0xD8, 6),
        ("fdiv", 8) => (0xDC, 6),
        ("fdivr", 4) => (0xD8, 7),
        _ => return Err(AsmError::UnknownMnemonic(name.to_string())),
    };

    emitter.encode_x87(opcode, digit, &rm)
}

fn two(emitter: &mut Emitter, a: u8, b: u8) -> Result<()> {
    emitter.byte(a);
    emitter.byte(b);
    Ok(())
}

fn sse(emitter: &mut Emitter, name: &str, ops: &[Operand]) -> Result<()> {
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

    let moves: &[(&str, Option<u8>, u8, u8)] = &[
        ("movss", Some(0xF3), 0x10, 0x11),
        ("movsd", Some(0xF2), 0x10, 0x11),
        ("movups", None, 0x10, 0x11),
        ("movaps", None, 0x28, 0x29),
        ("movapd", Some(0x66), 0x28, 0x29),
        ("movdqa", Some(0x66), 0x6F, 0x7F),
        ("movdqu", Some(0xF3), 0x6F, 0x7F),
    ];

    if let Some((_, prefix, load, store)) = moves.iter().find(|(m, _, _, _)| *m == name) {
        if let Some(reg) = dst.as_reg().filter(|r| r.class == Class::Xmm) {
            let rm = to_rm(src)?;
            return emitter.encode_sse(*prefix, &[0x0F, *load], reg, &rm, false);
        }
        let reg = src.as_reg().ok_or_else(|| bad(name))?;
        let rm = to_rm(dst)?;
        return emitter.encode_sse(*prefix, &[0x0F, *store], reg, &rm, false);
    }

    match name {
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
            if let Some(reg) = dst.as_reg().filter(|r| r.class == Class::Xmm) {
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
