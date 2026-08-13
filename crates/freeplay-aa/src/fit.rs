// does a table match the copy of the game on this disk
//
// aobscanmodule searches the main module, and the main module is the exe's
// code section, so the same bytes are there in the file. that means the whole
// question can be answered before the game is even started, which beats
// recording versions and waiting for somebody else to vote

use freeplay_core::pattern::Pattern;

use crate::script;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub symbol: String,
    pub pattern: String,
    // how many places in the exe it matched. one is what a table wants
    pub hits: usize,
    /* aobscan searches every readable page, not just the module, so a pattern
    it wants can legitimately live in a dll or on the heap. finding one in
    the exe still counts for the table, but not finding it proves nothing
    and must never be reported as a miss */
    pub anywhere: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    pub symbol: String,
    // what the script jumps to
    pub wants: u64,
    // where the branch in the bytes it restores actually goes
    pub goes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fit {
    pub signatures: Vec<Signature>,
    // scripts that jump to a fixed offset that disagrees with where the aob
    // landed. these assemble, patch cleanly and then crash the game
    pub stale: Vec<Stale>,
}

impl Fit {
    pub fn found(&self) -> usize {
        self.signatures.iter().filter(|s| s.hits == 1).count()
    }

    pub fn missing(&self) -> usize {
        self.signatures
            .iter()
            .filter(|s| s.hits == 0 && !s.anywhere)
            .count()
    }

    // scans we cannot answer from the file alone
    pub fn unknown(&self) -> usize {
        self.signatures
            .iter()
            .filter(|s| s.hits == 0 && s.anywhere)
            .count()
    }

    pub fn ambiguous(&self) -> usize {
        self.signatures.iter().filter(|s| s.hits > 1).count()
    }

    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty() && self.stale.is_empty()
    }
}

// the executable's code, and the address the first byte of it has once loaded
pub struct Code<'a> {
    pub bytes: &'a [u8],
    pub rva: u64,
}

/// One script against one exe.
pub fn of_script(source: &str, code: &Code) -> Fit {
    let Ok(halves) = script::parse(source) else {
        return Fit::default();
    };

    let mut fit = Fit::default();
    let mut landed: Vec<(String, u64)> = Vec::new();

    for half in [&halves.enable, &halves.disable] {
        for directive in &half.directives {
            let (symbol, pattern, anywhere) = match directive {
                script::Directive::AobScanModule {
                    symbol, pattern, ..
                } => (symbol, pattern, false),
                script::Directive::AobScan { symbol, pattern } => (symbol, pattern, true),
                _ => continue,
            };
            let Ok(parsed) = Pattern::parse(pattern) else {
                continue;
            };
            let hits = parsed.find_all(code.bytes);
            if hits.len() == 1 {
                landed.push((symbol.clone(), code.rva + hits[0] as u64));
            }
            fit.signatures.push(Signature {
                symbol: symbol.clone(),
                pattern: pattern.clone(),
                hits: hits.len(),
                anywhere,
            });
        }
    }

    fit.stale = stale(&halves, &landed);
    fit
}

/* a script that hooks a conditional branch has to send the branch on to where
it was already going. the author works that address out on their build and
writes it in as $process+something, which is only right on that build.

the disable half puts the original bytes back, so those bytes say where the
branch really went. compare the two and a table written for another version
of the game gives itself away. */
fn stale(halves: &script::Script, landed: &[(String, u64)]) -> Vec<Stale> {
    let mut found = Vec::new();

    for (symbol, at) in landed {
        let Some(original) = restored(halves, symbol) else {
            continue;
        };
        let Some(goes) = branch_target(&original, *at) else {
            continue;
        };
        for wants in fixed_targets(&halves.enable) {
            // the same landing spot written two ways is the healthy case
            if wants != goes && near(wants, goes) {
                found.push(Stale {
                    symbol: symbol.clone(),
                    wants,
                    goes,
                });
                break;
            }
        }
    }
    found
}

// only complain about a jump that was clearly meant for this hook. a script
// can legitimately jump somewhere else entirely in the module
fn near(wants: u64, goes: u64) -> bool {
    wants.abs_diff(goes) < 0x8000
}

// the `db ...` a section writes back over the hook, which is the original code
fn restored(halves: &script::Script, symbol: &str) -> Option<Vec<u8>> {
    let section = halves
        .disable
        .sections
        .iter()
        .find(|s| s.anchor.trim().eq_ignore_ascii_case(symbol))?;

    let mut bytes = Vec::new();
    for line in section.body.lines() {
        let text = line.trim();
        let Some(rest) = text
            .strip_prefix("db ")
            .or_else(|| text.strip_prefix("DB "))
        else {
            continue;
        };
        for token in rest.split(|c: char| c.is_whitespace() || c == ',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            match u8::from_str_radix(token, 16) {
                Ok(byte) => bytes.push(byte),
                // a wildcard or a string, so this is not a plain byte run
                Err(_) => return None,
            }
        }
    }
    (!bytes.is_empty()).then_some(bytes)
}

/* where the conditional branch inside the original bytes actually points.
there is no disassembler here, so this only trusts two shapes.

the six byte `0F 8x rel32` first, because that opcode pair is distinctive
enough to find by scanning. the two byte `7x rel8` only when it is the last
instruction in the run, since a lone 7x byte in the middle is far more
likely to be part of a modrm: `83 7E 18 00` is cmp dword ptr [esi+18],0 and
the 7E in it read as jle the first time this was written */
fn branch_target(original: &[u8], at: u64) -> Option<u64> {
    for i in 0..original.len().saturating_sub(5) {
        if original[i] == 0x0F && (0x80..=0x8F).contains(&original[i + 1]) {
            let rel = i32::from_le_bytes(original[i + 2..i + 6].try_into().ok()?);
            return Some((at + (i as u64) + 6).wrapping_add(rel as i64 as u64));
        }
    }

    let tail = original.len().checked_sub(2)?;
    if (0x70..=0x7F).contains(&original[tail]) {
        let rel = original[tail + 1] as i8;
        return Some((at + original.len() as u64).wrapping_add(rel as i64 as u64));
    }
    None
}

// every `$process+NNNN` an enable half jumps to, as an rva
fn fixed_targets(half: &script::Half) -> Vec<u64> {
    let mut found = Vec::new();
    for section in &half.sections {
        for line in section.body.lines() {
            let Some(rest) = after_process(line) else {
                continue;
            };
            if let Ok(value) = u64::from_str_radix(rest.trim(), 16) {
                found.push(value);
            }
        }
    }
    found
}

fn after_process(line: &str) -> Option<&str> {
    let lower = line.to_ascii_lowercase();
    let at = lower.find("$process+")?;
    let rest = &line[at + "$process+".len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    // 83 7E 18 00        cmp dword ptr [esi+18],0
    // 0F 8E 9E 01 00 00  jle +0x19e
    const HOOK: &[u8] = &[0x83, 0x7E, 0x18, 0x00, 0x0F, 0x8E, 0x9E, 0x01, 0x00, 0x00];

    fn code(bytes: &[u8]) -> Code<'_> {
        Code { bytes, rva: 0x1000 }
    }

    #[test]
    fn a_signature_that_is_there_counts_once() {
        let source = "[ENABLE]\naobscanmodule(hook,$process,83 7E 18 00 0F 8E)\n[DISABLE]\n";
        let fit = of_script(source, &code(HOOK));
        assert_eq!(fit.found(), 1);
        assert_eq!(fit.missing(), 0);
    }

    #[test]
    fn a_signature_for_another_build_is_missing() {
        let source = "[ENABLE]\naobscanmodule(hook,$process,11 22 33 44 55 66)\n[DISABLE]\n";
        let fit = of_script(source, &code(HOOK));
        assert_eq!(fit.missing(), 1);
        assert_eq!(fit.found(), 0);
    }

    #[test]
    fn wildcards_still_match() {
        let source = "[ENABLE]\naobscanmodule(hook,$process,83 7E ?? 00 0F 8E)\n[DISABLE]\n";
        assert_eq!(of_script(source, &code(HOOK)).found(), 1);
    }

    #[test]
    fn two_matches_are_not_one_match() {
        let twice = [HOOK, HOOK].concat();
        let source = "[ENABLE]\naobscanmodule(hook,$process,83 7E 18 00)\n[DISABLE]\n";
        let fit = of_script(source, &code(&twice));
        assert_eq!(fit.ambiguous(), 1);
        assert_eq!(fit.found(), 0);
    }

    /* the tomb raider table that crashed the game. the aob is fine and lands
    at 0x1000, so the jle inside the restored bytes goes to 0x11a8, and the
    script jumps to 0x1988 instead */
    #[test]
    fn a_jump_written_for_another_build_is_caught() {
        let source = "[ENABLE]\n\
                      aobscanmodule(endurance,$process,83 7E 18 00 0F 8E 9E 01 00 00)\n\
                      newmem:\n  jng $process+1988\n  jmp return\n\
                      endurance:\n  jmp newmem\n\
                      [DISABLE]\n\
                      endurance:\n  db 83 7E 18 00 0F 8E 9E 01 00 00\n";
        let fit = of_script(source, &code(HOOK));
        assert_eq!(fit.found(), 1, "the signature itself is fine");
        assert_eq!(
            fit.stale,
            vec![Stale {
                symbol: "endurance".into(),
                wants: 0x1988,
                goes: 0x11A8,
            }]
        );
    }

    #[test]
    fn a_jump_that_agrees_is_not_a_complaint() {
        let source = "[ENABLE]\n\
                      aobscanmodule(endurance,$process,83 7E 18 00 0F 8E 9E 01 00 00)\n\
                      newmem:\n  jng $process+11A8\n\
                      endurance:\n  jmp newmem\n\
                      [DISABLE]\n\
                      endurance:\n  db 83 7E 18 00 0F 8E 9E 01 00 00\n";
        let fit = of_script(source, &code(HOOK));
        assert!(fit.stale.is_empty(), "{:?}", fit.stale);
    }

    /* a script is allowed to jump somewhere else in the module for its own
    reasons, and that is not evidence of anything */
    #[test]
    fn a_jump_nowhere_near_the_hook_is_left_alone() {
        let source = "[ENABLE]\n\
                      aobscanmodule(endurance,$process,83 7E 18 00 0F 8E 9E 01 00 00)\n\
                      newmem:\n  call $process+9F0000\n\
                      endurance:\n  jmp newmem\n\
                      [DISABLE]\n\
                      endurance:\n  db 83 7E 18 00 0F 8E 9E 01 00 00\n";
        assert!(of_script(source, &code(HOOK)).stale.is_empty());
    }

    #[test]
    fn a_short_jcc_is_read_too() {
        // 85 C0        test eax,eax
        // 74 10        jz +0x10
        let bytes = &[0x85, 0xC0, 0x74, 0x10];
        let source = "[ENABLE]\n\
                      aobscanmodule(h,$process,85 C0 74 10)\n\
                      newmem:\n  jz $process+1200\n\
                      h:\n  jmp newmem\n\
                      [DISABLE]\n\
                      h:\n  db 85 C0 74 10\n";
        let fit = of_script(source, &code(bytes));
        assert_eq!(fit.stale.len(), 1);
        // 0x1000 + 2 + 2 + 0x10
        assert_eq!(fit.stale[0].goes, 0x1014);
    }

    #[test]
    fn a_script_with_no_scan_says_nothing() {
        let fit = of_script("[ENABLE]\nalloc(x,4)\n[DISABLE]\n", &code(HOOK));
        assert!(fit.is_empty());
    }

    #[test]
    fn rubbish_is_not_a_panic() {
        for source in ["", "[ENABLE]", "aobscanmodule(", "\0\0\0"] {
            let _ = of_script(source, &code(HOOK));
        }
    }
}
