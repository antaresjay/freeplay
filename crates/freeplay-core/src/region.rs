use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protection {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    /// Guard pages fault on first touch. Reading one would disturb the game,
    /// so the scanner skips them.
    pub guard: bool,
}

impl Protection {
    pub const NONE: Self = Self {
        read: false,
        write: false,
        execute: false,
        guard: false,
    };

    pub fn scannable(&self) -> bool {
        self.read && !self.guard
    }
}

impl fmt::Display for Protection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let flag = |on, c| if on { c } else { '-' };
        write!(
            f,
            "{}{}{}{}",
            flag(self.read, 'r'),
            flag(self.write, 'w'),
            flag(self.execute, 'x'),
            flag(self.guard, 'g')
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub base: usize,
    pub size: usize,
    pub protection: Protection,
    /// True for memory backed by a file, such as a loaded module. Game state
    /// lives in private memory, so a value scan can skip these.
    pub mapped: bool,
}

impl Region {
    pub fn end(&self) -> usize {
        self.base.saturating_add(self.size)
    }

    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.end()
    }

    /// Worth searching for a changing gameplay value.
    pub fn scannable_data(&self) -> bool {
        self.protection.scannable() && self.protection.write && !self.mapped
    }

    /// Worth searching for a code signature.
    pub fn scannable_code(&self) -> bool {
        self.protection.scannable() && self.protection.execute
    }
}
