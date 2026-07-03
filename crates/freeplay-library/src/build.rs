//! the version stamped into a game's exe
//!
//! a table is written against one build and quietly stops resolving on the
//! next. saying which build somebody checked it on is the difference between
//! "this is broken" and "this is for the version before yours"

use std::path::Path;

#[cfg(windows)]
pub fn of(exe: &Path) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };

    let wide: Vec<u16> = exe
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let path = PCWSTR(wide.as_ptr());

    let size = unsafe { GetFileVersionInfoSizeW(path, None) };
    if size == 0 {
        return None;
    }

    let mut block = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(path, None, size, block.as_mut_ptr().cast()) }.ok()?;

    let mut found = std::ptr::null_mut();
    let mut length = 0u32;
    let root: Vec<u16> = "\\".encode_utf16().chain(std::iter::once(0)).collect();

    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            PCWSTR(root.as_ptr()),
            &mut found,
            &mut length,
        )
    };
    if !ok.as_bool()
        || found.is_null()
        || (length as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
    {
        return None;
    }

    let info = unsafe { &*(found as *const VS_FIXEDFILEINFO) };
    // stored as two dwords, high word then low word of each
    let version = format!(
        "{}.{}.{}.{}",
        info.dwFileVersionMS >> 16,
        info.dwFileVersionMS & 0xFFFF,
        info.dwFileVersionLS >> 16,
        info.dwFileVersionLS & 0xFFFF
    );

    // plenty of games ship 0.0.0.0, which tells nobody anything
    (version != "0.0.0.0").then_some(version)
}

#[cfg(not(windows))]
pub fn of(_exe: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_there_is_none_not_a_panic() {
        assert_eq!(of(Path::new("Z:/nothing/here.exe")), None);
    }

    #[cfg(windows)]
    #[test]
    fn reads_the_version_off_something_windows_ships() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:/Windows".into());
        let notepad = std::path::PathBuf::from(root)
            .join("System32")
            .join("notepad.exe");
        if !notepad.is_file() {
            return;
        }
        let found = of(&notepad).expect("notepad carries a version");
        assert_eq!(found.split('.').count(), 4, "got {found}");
    }

    #[cfg(windows)]
    #[test]
    fn a_file_with_no_version_block_is_none() {
        let scratch = std::env::temp_dir().join("freeplay-build-test.exe");
        std::fs::write(&scratch, b"not really an exe").unwrap();
        assert_eq!(of(&scratch), None);
        let _ = std::fs::remove_file(&scratch);
    }
}
