// the code section of an exe, straight off the disk
//
// a table's aobscanmodule searches the game's main module once it is loaded.
// the loader maps the file's code section without changing it, so the same
// bytes are in the file, and a table can be checked against a game that is not
// even running

use std::path::Path;

pub struct Code {
    pub bytes: Vec<u8>,
    // where the first byte lands once the module is loaded, relative to the
    // module base. an aob hit at index n sits at base + rva + n
    pub rva: u64,
    pub bits: u32,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The first executable section, which is where every scan of ours lands.
pub fn code(exe: &Path) -> Option<Code> {
    let data = std::fs::read(exe).ok()?;
    from_bytes(&data)
}

fn from_bytes(data: &[u8]) -> Option<Code> {
    if data.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(data, 0x3C)? as usize;
    if data.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }

    let sections = u16_at(data, pe + 6)? as usize;
    let optional = u16_at(data, pe + 20)? as usize;
    let bits = match u16_at(data, pe + 24)? {
        0x10B => 32,
        0x20B => 64,
        _ => return None,
    };

    // a packed or protected exe decrypts itself at startup, so what is on disk
    // is not what gets scanned. no way to tell that from here, which is why
    // the caller has to treat a miss as "cannot say" rather than "not there"
    let table = pe + 24 + optional;
    for i in 0..sections {
        let at = table + i * 40;
        let flags = u32_at(data, at + 36)?;
        // IMAGE_SCN_MEM_EXECUTE
        if flags & 0x2000_0000 == 0 {
            continue;
        }
        let virtual_address = u32_at(data, at + 12)? as u64;
        let raw_size = u32_at(data, at + 16)? as usize;
        let raw = u32_at(data, at + 20)? as usize;
        let bytes = data.get(raw..raw.checked_add(raw_size)?)?.to_vec();
        if bytes.is_empty() {
            continue;
        }
        return Some(Code {
            bytes,
            rva: virtual_address,
            bits,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // enough of a pe for the reader, with one executable section
    fn fake(bits: u16, code_at: u32, code: &[u8]) -> Vec<u8> {
        let optional = 96usize;
        let pe = 0x80usize;
        let table = pe + 24 + optional;
        let raw = 0x400usize;
        let mut data = vec![0u8; raw + code.len()];

        data[0..2].copy_from_slice(b"MZ");
        data[0x3C..0x40].copy_from_slice(&(pe as u32).to_le_bytes());
        data[pe..pe + 4].copy_from_slice(b"PE\0\0");
        data[pe + 6..pe + 8].copy_from_slice(&1u16.to_le_bytes());
        data[pe + 20..pe + 22].copy_from_slice(&(optional as u16).to_le_bytes());
        data[pe + 24..pe + 26].copy_from_slice(&bits.to_le_bytes());

        data[table..table + 5].copy_from_slice(b".text");
        data[table + 12..table + 16].copy_from_slice(&code_at.to_le_bytes());
        data[table + 16..table + 20].copy_from_slice(&(code.len() as u32).to_le_bytes());
        data[table + 20..table + 24].copy_from_slice(&(raw as u32).to_le_bytes());
        data[table + 36..table + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());

        data[raw..raw + code.len()].copy_from_slice(code);
        data
    }

    #[test]
    fn reads_the_code_section_and_where_it_lands() {
        let found = from_bytes(&fake(0x10B, 0x1000, &[0x90, 0xCC, 0x90])).unwrap();
        assert_eq!(found.bytes, vec![0x90, 0xCC, 0x90]);
        assert_eq!(found.rva, 0x1000);
        assert_eq!(found.bits, 32);
    }

    #[test]
    fn knows_a_64_bit_one() {
        assert_eq!(from_bytes(&fake(0x20B, 0x1000, &[0x90])).unwrap().bits, 64);
    }

    #[test]
    fn a_section_that_does_not_execute_is_not_the_code() {
        let mut data = fake(0x10B, 0x1000, &[0x90]);
        let table = 0x80 + 24 + 96;
        // IMAGE_SCN_MEM_READ only
        data[table + 36..table + 40].copy_from_slice(&0x4000_0040u32.to_le_bytes());
        assert!(from_bytes(&data).is_none());
    }

    #[test]
    fn anything_that_is_not_a_pe_is_none_rather_than_a_panic() {
        for junk in [
            b"".to_vec(),
            b"MZ".to_vec(),
            b"not an exe at all".to_vec(),
            vec![0u8; 512],
        ] {
            assert!(from_bytes(&junk).is_none());
        }
    }

    #[test]
    fn a_truncated_file_is_none_rather_than_a_panic() {
        let whole = fake(0x10B, 0x1000, &[0x90; 32]);
        for cut in [4, 0x40, 0x84, 0x100, whole.len() - 8] {
            let _ = from_bytes(&whole[..cut]);
        }
    }

    /* the reader is worth nothing if it cannot read the real thing, so it gets
    pointed at a binary this machine definitely has */
    #[test]
    #[cfg(windows)]
    fn reads_a_real_windows_binary() {
        let root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| r"C:\Windows".into());
        let path = std::path::PathBuf::from(root)
            .join("System32")
            .join("notepad.exe");
        if !path.is_file() {
            return;
        }
        let found = code(&path).expect("notepad has a code section");
        assert!(found.bytes.len() > 4096, "{} bytes", found.bytes.len());
        assert!(found.rva >= 0x1000);
        assert_eq!(found.bits, 64);
    }
}
