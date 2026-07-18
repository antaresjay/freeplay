use thiserror::Error;

#[derive(Debug, Error)]
pub enum AaError {
    #[error("line {line}: {text:?} is not under any label, so there is nowhere to put it")]
    Stray { line: usize, text: String },

    #[error("cannot read the directive {0:?}")]
    BadDirective(String),

    #[error("this one is written in {0}, not assembly, and Freeplay only runs assembly")]
    NotAssembly(&'static str),

    #[error("{0} is used but never scanned for, allocated or labelled")]
    UndefinedSymbol(String),

    #[error("the signature for {symbol} is not in this build of the game")]
    SignatureMissing { symbol: String },

    #[error("the bytes at {symbol} are not what the script expects, so the game has been patched")]
    AssertFailed { symbol: String },

    #[error("{symbol} needs {wanted} bytes of room but the original code there is {room}")]
    NoRoom {
        symbol: String,
        wanted: usize,
        room: usize,
    },

    #[error("that table does not get to do this: {}", .refusals.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))]
    Refused {
        refusals: Vec<crate::safety::Refusal>,
    },

    #[error(transparent)]
    Assembly(#[from] freeplay_asm::AsmError),

    #[error(transparent)]
    Target(#[from] freeplay_core::Error),
}

pub type Result<T> = std::result::Result<T, AaError>;
