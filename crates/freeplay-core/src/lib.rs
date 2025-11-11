//! Reading, scanning and patching another process's memory.
//!
//! Everything platform specific lives behind [`Target`]. The scanner, pointer
//! chains and patch logic sit on top of that trait and never call an OS API
//! directly, so a Linux backend using process_vm_readv is a new module rather
//! than a rewrite.

pub mod error;
pub mod guard;
pub mod region;
pub mod target;
pub mod value;

#[cfg(windows)]
pub mod windows_target;

pub use error::{Error, Result};
pub use region::{Protection, Region};
pub use target::{Module, Target};
pub use value::{Scalar, ValueKind};

#[cfg(windows)]
pub use windows_target::WindowsTarget;

/// The concrete target for this build.
#[cfg(windows)]
pub type NativeTarget = WindowsTarget;
