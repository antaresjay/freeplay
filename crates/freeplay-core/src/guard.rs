//! Anti-cheat detection.
//!
//! Freeplay is for single player games. Attaching to something with an
//! anti-cheat running gets people banned from games they paid for, and the
//! whole point of a trainer is to mess about in your own save, not to spoil
//! someone else's match. This check runs before any handle is used for writing
//! and it is not optional.

use crate::target::Module;

/// Substring of a loaded module name, and the product it belongs to. Matching
/// is on a lowercased substring because vendors ship the same DLL under
/// several names (EasyAntiCheat.dll, EasyAntiCheat_x64.dll, and so on).
const PROTECTED_MODULES: &[(&str, &str)] = &[
    ("easyanticheat", "EasyAntiCheat"),
    ("eac_", "EasyAntiCheat"),
    ("beclient", "BattlEye"),
    ("battleye", "BattlEye"),
    ("vgc", "Riot Vanguard"),
    ("vgk", "Riot Vanguard"),
    ("gameguard", "nProtect GameGuard"),
    ("npggnt", "nProtect GameGuard"),
    ("xhunter", "XIGNCODE3"),
    ("xigncode", "XIGNCODE3"),
    ("hshield", "AhnLab HackShield"),
    ("denuvo", "Denuvo Anti-Cheat"),
    ("anticheat", "an anti-cheat"),
    ("punkbuster", "PunkBuster"),
    ("pbcl", "PunkBuster"),
    ("mhyprot", "mihoyo anti-cheat"),
    ("faceit", "FACEIT AC"),
    ("esea", "ESEA AC"),
];

/// Processes we refuse to touch even if no anti-cheat module is loaded yet,
/// because they are the anti-cheat.
const PROTECTED_PROCESSES: &[&str] = &[
    "easyanticheat.exe",
    "easyanticheat_eos.exe",
    "beservice.exe",
    "vgtray.exe",
    "vanguard.exe",
];

const GENERIC: &str = "an anti-cheat";

// a hit, and the name that gave it away. the caller hangs on to the second
// half so it can read that file's version block when the first half is empty
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spotted {
    // none when it is plainly an anti-cheat but the file name does not say whose
    pub product: Option<&'static str>,
    pub found_in: String,
}

// public because the same list names a product read out of a version block
pub fn product_for(lowered: &str) -> Option<&'static str> {
    let mut hit = None;
    for (needle, product) in PROTECTED_MODULES {
        if lowered.contains(needle) {
            // Keep looking so a specific product wins over the generic
            // "anticheat" catch-all in the message.
            if *product == GENERIC {
                hit.get_or_insert(*product);
            } else {
                return Some(product);
            }
        }
    }
    hit
}

pub fn inspect_modules(modules: &[Module]) -> Option<&'static str> {
    inspect_names(modules.iter().map(|m| m.name.as_str()))
}

/// The same list applied to plain names, so the interface can mark a game as
/// off limits from what is sitting in its install folder rather than waiting
/// for somebody to try attaching. This is a hint only. The refusal that counts
/// runs against modules the process has actually loaded.
pub fn inspect_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'static str> {
    look(names).map(|found| found.product.unwrap_or(GENERIC))
}

// the same sweep, keeping hold of which name matched
pub fn look<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<Spotted> {
    let mut vague = None;
    for name in names {
        match product_for(&name.to_ascii_lowercase()) {
            Some(GENERIC) => {
                vague.get_or_insert_with(|| Spotted {
                    product: None,
                    found_in: name.to_string(),
                });
            }
            Some(product) => {
                return Some(Spotted {
                    product: Some(product),
                    found_in: name.to_string(),
                })
            }
            None => {}
        }
    }
    vague
}

pub fn is_protected_process(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PROTECTED_PROCESSES.iter().any(|p| lower == *p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(name: &str) -> Module {
        Module {
            name: name.into(),
            base: 0,
            size: 0,
        }
    }

    #[test]
    fn clean_process_passes() {
        let modules = [
            module("ntdll.dll"),
            module("MassEffect1.exe"),
            module("d3d11.dll"),
        ];
        assert_eq!(inspect_modules(&modules), None);
    }

    #[test]
    fn spots_easy_anti_cheat_under_any_name() {
        assert_eq!(
            inspect_modules(&[module("EasyAntiCheat_x64.dll")]),
            Some("EasyAntiCheat")
        );
        assert_eq!(
            inspect_modules(&[module("easyanticheat.dll")]),
            Some("EasyAntiCheat")
        );
    }

    #[test]
    fn spots_battleye() {
        assert_eq!(
            inspect_modules(&[module("BEClient_x64.dll")]),
            Some("BattlEye")
        );
    }

    #[test]
    fn named_product_beats_the_generic_match() {
        let modules = [module("someanticheat.dll"), module("BEClient.dll")];
        assert_eq!(inspect_modules(&modules), Some("BattlEye"));
    }

    #[test]
    fn generic_match_still_refuses() {
        assert_eq!(
            inspect_modules(&[module("gameanticheat64.dll")]),
            Some("an anti-cheat")
        );
    }

    #[test]
    fn spots_an_anti_cheat_sitting_in_the_install_folder() {
        let files = [
            "Discovery.exe",
            "EasyAntiCheat",
            "Engine",
            "steam_api64.dll",
        ];
        assert_eq!(inspect_names(files), Some("EasyAntiCheat"));
    }

    #[test]
    fn an_ordinary_game_folder_is_left_alone() {
        let files = [
            "witcher2.exe",
            "CookedPC",
            "bin",
            "UserContentTools",
            "Release",
        ];
        assert_eq!(inspect_names(files), None);
    }

    #[test]
    fn keeps_hold_of_the_name_that_matched() {
        let found = look(["Discovery.exe", "AntiCheatInstaller.exe"]).unwrap();
        assert_eq!(found.product, None, "the file name does not say whose");
        assert_eq!(found.found_in, "AntiCheatInstaller.exe");

        let named = look(["EasyAntiCheat_x64.dll"]).unwrap();
        assert_eq!(named.product, Some("EasyAntiCheat"));
        assert_eq!(named.found_in, "EasyAntiCheat_x64.dll");
    }

    #[test]
    fn a_named_product_wins_over_a_vague_one_found_first() {
        let found = look(["someanticheat.dll", "BEClient.dll"]).unwrap();
        assert_eq!(found.product, Some("BattlEye"));
    }

    #[test]
    fn a_version_block_string_runs_through_the_same_list() {
        assert_eq!(
            product_for(&"Denuvo Anti-Cheat Installer".to_ascii_lowercase()),
            Some("Denuvo Anti-Cheat")
        );
    }

    #[test]
    fn refuses_the_anti_cheat_service_itself() {
        assert!(is_protected_process("EasyAntiCheat.exe"));
        assert!(!is_protected_process("MassEffect1.exe"));
    }
}
