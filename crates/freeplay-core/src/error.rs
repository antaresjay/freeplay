pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no running process named {0}")]
    ProcessNotFound(String),

    #[error("could not open process {pid}, try running as administrator: {source}")]
    OpenFailed {
        pid: u32,
        #[source]
        source: std::io::Error,
    },

    /// Freeplay will not attach to anything running an anti-cheat. Cheating in
    /// multiplayer ruins other people's games and gets accounts banned.
    #[error("{process} is running {guard}, refusing to attach")]
    Protected { process: String, guard: &'static str },

    #[error("read of {len} bytes at {addr:#x} failed: {source}")]
    ReadFailed {
        addr: usize,
        len: usize,
        #[source]
        source: std::io::Error,
    },

    #[error("write of {len} bytes at {addr:#x} failed: {source}")]
    WriteFailed {
        addr: usize,
        len: usize,
        #[source]
        source: std::io::Error,
    },

    #[error("module {0} is not loaded in the target")]
    ModuleNotFound(String),

    #[error("bad byte pattern: {0}")]
    BadPattern(String),

    #[error("pattern matched {found} times, expected one")]
    Ambiguous { found: usize },

    #[error("pattern not found, the game has probably been patched")]
    NotFound,

    #[error("pointer chain broke at hop {hop}, address {addr:#x}")]
    BrokenChain { hop: usize, addr: usize },

    #[error("target is 32-bit, freeplay only drives 64-bit processes")]
    ArchMismatch,

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
