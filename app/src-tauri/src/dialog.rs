//! the windows file picker, straight from comdlg32. a plugin for this would
//! pull in a stack of dependencies for two calls

use std::path::PathBuf;

pub struct Ask<'a> {
    pub title: &'a str,
    // pairs of what to call it and what to match, ("Cheat Engine table", "*.CT")
    pub kinds: &'a [(&'a str, &'a str)],
    pub suggested: &'a str,
    pub extension: &'a str,
}

#[cfg(windows)]
mod windows_picker {
    use super::{Ask, PathBuf};

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
        OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // one flat block of "name\0pattern\0name\0pattern\0\0", which is how the
    // api has wanted it since 1995
    fn filter_for(kinds: &[(&str, &str)]) -> Vec<u16> {
        let mut out = Vec::new();
        for (label, pattern) in kinds {
            out.extend(format!("{label} ({pattern})").encode_utf16());
            out.push(0);
            out.extend(pattern.encode_utf16());
            out.push(0);
        }
        out.push(0);
        out
    }

    fn run(ask: &Ask<'_>, saving: bool) -> Option<PathBuf> {
        let mut buffer = vec![0u16; 2048];
        for (at, unit) in wide(ask.suggested).iter().enumerate() {
            if at < buffer.len() - 1 {
                buffer[at] = *unit;
            }
        }

        let filter = filter_for(ask.kinds);
        let title = wide(ask.title);
        let extension = wide(ask.extension);

        let mut args = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(buffer.as_mut_ptr()),
            nMaxFile: buffer.len() as u32,
            lpstrTitle: PCWSTR(title.as_ptr()),
            lpstrDefExt: PCWSTR(extension.as_ptr()),
            Flags: if saving {
                OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY
            } else {
                OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY
            },
            ..Default::default()
        };

        let chose = unsafe {
            if saving {
                GetSaveFileNameW(&mut args)
            } else {
                GetOpenFileNameW(&mut args)
            }
        };
        if !chose.as_bool() {
            return None;
        }

        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        let picked = String::from_utf16_lossy(&buffer[..end]);
        (!picked.is_empty()).then(|| PathBuf::from(picked))
    }

    pub fn open(ask: &Ask<'_>) -> Option<PathBuf> {
        run(ask, false)
    }

    pub fn save(ask: &Ask<'_>) -> Option<PathBuf> {
        run(ask, true)
    }
}

#[cfg(windows)]
pub use windows_picker::{open, save};

#[cfg(not(windows))]
pub fn open(_ask: &Ask<'_>) -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
pub fn save(_ask: &Ask<'_>) -> Option<PathBuf> {
    None
}

pub const TABLES: &[(&str, &str)] = &[("Cheat Engine table", "*.CT")];
pub const PROFILES: &[(&str, &str)] = &[("Freeplay profile", "*.freeplay")];
pub const TEXT: &[(&str, &str)] = &[("Text file", "*.txt")];
