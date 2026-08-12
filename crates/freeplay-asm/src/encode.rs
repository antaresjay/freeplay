use crate::error::{AsmError, Result};
use crate::operand::{Class, Mem, Reg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bits {
    X86,
    X64,
}

impl Bits {
    pub fn pointer(self) -> usize {
        match self {
            Bits::X86 => 4,
            Bits::X64 => 8,
        }
    }
}

pub struct Emitter {
    pub bits: Bits,
    pub origin: u64,
    pub bytes: Vec<u8>,
    pub fixups: Vec<Fixup>,
    pub holes: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct Fixup {
    pub at: usize,
    pub width: usize,
    pub symbol: String,
    pub relative_to_end: Option<usize>,
    pub addend: i64,
}

impl Emitter {
    pub fn new(bits: Bits, origin: u64) -> Self {
        Self {
            bits,
            origin,
            bytes: Vec::new(),
            fixups: Vec::new(),
            holes: Vec::new(),
        }
    }

    pub fn here(&self) -> u64 {
        self.origin + self.bytes.len() as u64
    }

    pub fn byte(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn bytes_from(&mut self, data: &[u8]) {
        self.bytes.extend_from_slice(data);
    }

    pub fn word(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn dword(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn qword(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn immediate(&mut self, value: i64, width: usize) {
        match width {
            1 => self.byte(value as u8),
            2 => self.word(value as u16),
            4 => self.dword(value as u32),
            8 => self.qword(value as u64),
            _ => {}
        }
    }

    pub fn fixup(&mut self, symbol: &str, width: usize, addend: i64, relative: bool) {
        let at = self.bytes.len();
        self.fixups.push(Fixup {
            at,
            width,
            symbol: symbol.to_string(),
            relative_to_end: if relative { Some(0) } else { None },
            addend,
        });
        for _ in 0..width {
            self.bytes.push(0);
        }
    }

    pub fn close_relative_fixups(&mut self) {
        let end = self.bytes.len();
        for fixup in self.fixups.iter_mut() {
            if fixup.relative_to_end == Some(0) && fixup.at + fixup.width <= end {
                fixup.relative_to_end = Some(end);
            }
        }
    }

    fn prefix_operand_size(&mut self, size: usize) {
        if size == 2 {
            self.byte(0x66);
        }
    }

    fn segment(&mut self, mem: &Mem) {
        if let Some(which) = mem.seg {
            self.byte(match which {
                0 => 0x26,
                1 => 0x2E,
                2 => 0x36,
                3 => 0x3E,
                4 => 0x64,
                _ => 0x65,
            });
        }
    }

    fn rex(&mut self, wide: bool, reg: u8, index: u8, base: u8, force: bool) {
        if self.bits != Bits::X64 {
            return;
        }
        let mut value = 0x40u8;
        if wide {
            value |= 0x08;
        }
        if reg >= 8 {
            value |= 0x04;
        }
        if index >= 8 {
            value |= 0x02;
        }
        if base >= 8 {
            value |= 0x01;
        }
        if value != 0x40 || force {
            self.byte(value);
        }
    }

    pub fn modrm_reg_reg(&mut self, reg: Reg, rm: Reg) {
        self.byte(0xC0 | (reg.low3() << 3) | rm.low3());
    }

    pub fn modrm_digit_reg(&mut self, digit: u8, rm: Reg) {
        self.byte(0xC0 | ((digit & 7) << 3) | rm.low3());
    }

    pub fn modrm_mem(&mut self, reg_field: u8, mem: &Mem) -> Result<()> {
        let reg_field = reg_field & 7;

        if mem.is_absolute() {
            if self.bits == Bits::X64 {
                self.byte((reg_field << 3) | 5);
                if let Some(symbol) = &mem.symbol {
                    self.fixup(symbol, 4, mem.disp, true);
                } else {
                    self.dword(mem.disp as u32);
                }
            } else {
                self.byte((reg_field << 3) | 5);
                if let Some(symbol) = &mem.symbol {
                    self.fixup(symbol, 4, mem.disp, false);
                } else {
                    self.dword(mem.disp as u32);
                }
            }
            return Ok(());
        }

        let base = mem.base;
        let index = mem.index;
        let has_symbol = mem.symbol.is_some();

        let needs_sib =
            index.is_some() || base.map(|b| b.low3() == 4).unwrap_or(false) || base.is_none();

        let disp_size = if has_symbol {
            4
        } else if mem.disp == 0 && base.map(|b| b.low3() != 5).unwrap_or(false) {
            0
        } else if (-128..=127).contains(&mem.disp) && base.is_some() {
            1
        } else {
            4
        };

        let mode = match disp_size {
            0 => 0b00,
            1 => 0b01,
            _ => 0b10,
        };

        if needs_sib {
            self.byte((mode << 6) | (reg_field << 3) | 4);
            let scale_bits = match mem.scale {
                2 => 1,
                4 => 2,
                8 => 3,
                _ => 0,
            };
            let index_bits = index.map(|r| r.low3()).unwrap_or(4);
            let base_bits = base.map(|r| r.low3()).unwrap_or(5);
            self.byte((scale_bits << 6) | (index_bits << 3) | base_bits);
        } else {
            let base_reg = base.ok_or_else(|| AsmError::Operand("no base".into()))?;
            self.byte((mode << 6) | (reg_field << 3) | base_reg.low3());
        }

        match (has_symbol, disp_size) {
            (true, _) => {
                let symbol = mem.symbol.clone().unwrap();
                self.fixup(&symbol, 4, mem.disp, false);
            }
            (false, 1) => self.byte(mem.disp as u8),
            (false, 4) => self.dword(mem.disp as u32),
            _ => {}
        }
        Ok(())
    }

    pub fn encode_rm(&mut self, opcode: &[u8], reg_field: u8, rm: &Rm, size: usize) -> Result<()> {
        self.prefix_operand_size(size);
        let wide = size == 8;
        match rm {
            Rm::Reg(r) => {
                let force = size == 1 && matches!(r.class, Class::Gpr8Rex);
                self.rex(wide, reg_field, 0, r.num, force);
                for byte in opcode {
                    self.byte(*byte);
                }
                self.byte(0xC0 | ((reg_field & 7) << 3) | r.low3());
            }
            Rm::Mem(m) => {
                let index = m.index.map(|r| r.num).unwrap_or(0);
                let base = m.base.map(|r| r.num).unwrap_or(0);
                self.segment(m);
                self.rex(wide, reg_field, index, base, false);
                for byte in opcode {
                    self.byte(*byte);
                }
                self.modrm_mem(reg_field, m)?;
            }
        }
        Ok(())
    }

    pub fn encode_reg_rm(&mut self, opcode: &[u8], reg: Reg, rm: &Rm, size: usize) -> Result<()> {
        self.prefix_operand_size(size);
        let wide = size == 8 || reg.class == Class::Gpr64;
        match rm {
            Rm::Reg(r) => {
                let force = size == 1
                    && (matches!(r.class, Class::Gpr8Rex) || matches!(reg.class, Class::Gpr8Rex));
                self.rex(wide, reg.num, 0, r.num, force);
                for byte in opcode {
                    self.byte(*byte);
                }
                self.byte(0xC0 | (reg.low3() << 3) | r.low3());
            }
            Rm::Mem(m) => {
                let index = m.index.map(|r| r.num).unwrap_or(0);
                let base = m.base.map(|r| r.num).unwrap_or(0);
                let force = size == 1 && matches!(reg.class, Class::Gpr8Rex);
                self.segment(m);
                self.rex(wide, reg.num, index, base, force);
                for byte in opcode {
                    self.byte(*byte);
                }
                self.modrm_mem(reg.low3(), m)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Rm {
    Reg(Reg),
    Mem(Mem),
}

impl Rm {
    pub fn size_hint(&self) -> Option<usize> {
        match self {
            Rm::Reg(r) => Some(r.size()),
            Rm::Mem(m) => m.size,
        }
    }

    pub fn as_reg(&self) -> Option<Reg> {
        match self {
            Rm::Reg(r) => Some(*r),
            Rm::Mem(_) => None,
        }
    }
}

impl Emitter {
    pub fn encode_sse(
        &mut self,
        prefix: Option<u8>,
        opcode: &[u8],
        reg: Reg,
        rm: &Rm,
        wide: bool,
    ) -> Result<()> {
        if let Some(p) = prefix {
            self.byte(p);
        }
        match rm {
            Rm::Reg(r) => {
                self.rex(wide, reg.num, 0, r.num, false);
                for byte in opcode {
                    self.byte(*byte);
                }
                self.byte(0xC0 | (reg.low3() << 3) | r.low3());
            }
            Rm::Mem(m) => {
                let index = m.index.map(|r| r.num).unwrap_or(0);
                let base = m.base.map(|r| r.num).unwrap_or(0);
                self.segment(m);
                self.rex(wide, reg.num, index, base, false);
                for byte in opcode {
                    self.byte(*byte);
                }
                self.modrm_mem(reg.low3(), m)?;
            }
        }
        Ok(())
    }

    /* the avx form of an sse instruction. same opcode, but the 66/f2/f3 and
    the 0f escape fold into a two or three byte prefix, and there is room in
    it for a second source register.

    two bytes will do unless something needs the extra bits: an x or b beyond
    the eighth register, a wide operand, or an escape other than plain 0f. */
    pub fn encode_vex(
        &mut self,
        prefix: Option<u8>,
        opcode: &[u8],
        reg: Reg,
        vvvv: Option<Reg>,
        rm: &Rm,
        wide: bool,
    ) -> Result<()> {
        let pp = match prefix {
            None => 0,
            Some(0x66) => 1,
            Some(0xF3) => 2,
            _ => 3,
        };
        let mm = match opcode {
            [0x0F, 0x38, ..] => 2,
            [0x0F, 0x3A, ..] => 3,
            _ => 1,
        };
        let tail = &opcode[if mm == 1 { 1 } else { 2 }..];

        let (index, base) = match rm {
            Rm::Reg(r) => (0, r.num),
            Rm::Mem(m) => (
                m.index.map(|r| r.num).unwrap_or(0),
                m.base.map(|r| r.num).unwrap_or(0),
            ),
        };
        if let Rm::Mem(m) = rm {
            self.segment(m);
        }

        let not = |bit: bool| if bit { 0 } else { 1 };
        let vpart = (!vvvv.map(|r| r.num).unwrap_or(0) & 0x0F) << 3;
        // one bit says whether this is the 128 or the 256 bit wide form
        let long = [Some(reg), vvvv, rm.as_reg()]
            .iter()
            .any(|r| r.is_some_and(|r| r.class == Class::Ymm)) as u8;

        if mm == 1 && index < 8 && base < 8 && !wide {
            self.byte(0xC5);
            self.byte((not(reg.num >= 8) << 7) | vpart | (long << 2) | pp);
        } else {
            self.byte(0xC4);
            self.byte(
                (not(reg.num >= 8) << 7) | (not(index >= 8) << 6) | (not(base >= 8) << 5) | mm,
            );
            self.byte(((wide as u8) << 7) | vpart | (long << 2) | pp);
        }

        for byte in tail {
            self.byte(*byte);
        }
        match rm {
            Rm::Reg(r) => self.byte(0xC0 | (reg.low3() << 3) | r.low3()),
            Rm::Mem(m) => self.modrm_mem(reg.low3(), m)?,
        }
        Ok(())
    }

    pub fn encode_x87(&mut self, opcode: u8, digit: u8, rm: &Rm) -> Result<()> {
        match rm {
            Rm::Mem(m) => {
                let index = m.index.map(|r| r.num).unwrap_or(0);
                let base = m.base.map(|r| r.num).unwrap_or(0);
                self.segment(m);
                self.rex(false, digit, index, base, false);
                self.byte(opcode);
                self.modrm_mem(digit, m)?;
                Ok(())
            }
            Rm::Reg(_) => Err(AsmError::Operand("x87 wants memory".into())),
        }
    }
}
