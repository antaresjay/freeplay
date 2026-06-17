use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AsmError {
    #[error("cannot make sense of the operand {0:?}")]
    Operand(String),

    #[error("{0} is not an instruction freeplay knows how to assemble")]
    UnknownMnemonic(String),

    #[error("{mnemonic} does not take those operands")]
    BadOperands { mnemonic: String },

    #[error("{0} is not defined")]
    UndefinedSymbol(String),

    #[error("{0} is defined twice")]
    DuplicateSymbol(String),

    #[error("{target:#x} is too far from {from:#x} to jump to")]
    OutOfRange { from: u64, target: u64 },

    #[error("line {line}: {source}")]
    At {
        line: usize,
        #[source]
        source: Box<AsmError>,
    },
}

pub type Result<T> = std::result::Result<T, AsmError>;
