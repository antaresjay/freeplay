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

pub fn inspect_modules(modules: &[Module]) -> Option<&'static str> {
    let mut hit = None;
    for module in modules {
        let name = module.name.to_ascii_lowercase();
        for (needle, product) in PROTECTED_MODULES {
            if name.contains(needle) {
                // Keep looking so a specific product wins over the generic
                // "anticheat" catch-all in the message.
                if *product == "an anti-cheat" {
                    hit.get_or_insert(*product);
                } else {
                    return Some(product);
                }
            }
        }
    }
    hit
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
    fn refuses_the_anti_cheat_service_itself() {
        assert!(is_protected_process("EasyAntiCheat.exe"));
        assert!(!is_protected_process("MassEffect1.exe"));
    }
}
