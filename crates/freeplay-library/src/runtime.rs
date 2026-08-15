//! what engine a game runs on, read off the modules it has loaded
//!
//! unity tables lean on cheat engine's mono dissector, which walks the game's
//! own mono runtime to turn `Namespace:Class:field` into an address. freeplay
//! does not drive that runtime, so the most useful thing it can do is name it:
//! a table that needs mono is a unity game, not a broken table, and saying so
//! is better than a shrug.
//!
//! this only reads the module list. nothing is called inside the game.

/// The runtime a loaded module list points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// Unity with the Mono backend. The dissector can walk this in Cheat
    /// Engine; freeplay cannot yet.
    Mono,
    /// Unity compiled through IL2CPP. No managed runtime to walk at all, the
    /// metadata is baked into GameAssembly.dll.
    Il2cpp,
    /// Anything else, which is everything freeplay handles today.
    Native,
}

impl Runtime {
    pub fn label(self) -> &'static str {
        match self {
            Runtime::Mono => "Unity (Mono)",
            Runtime::Il2cpp => "Unity (IL2CPP)",
            Runtime::Native => "native",
        }
    }

    pub fn is_unity(self) -> bool {
        matches!(self, Runtime::Mono | Runtime::Il2cpp)
    }
}

// the runtime dll ships under a handful of names across unity versions. all
// lowercased before the compare
const MONO: &[&str] = &[
    "mono.dll",
    "mono-2.0-bdwgc.dll",
    "monobleedingedge.dll",
    "mono-2.0-boehm.dll",
];

/// Work out the runtime from the names of the modules a process has loaded.
/// IL2CPP wins over Mono when both look present, because a game is one or the
/// other and GameAssembly is the definite tell.
pub fn of<'a>(module_names: impl IntoIterator<Item = &'a str>) -> Runtime {
    let mut mono = false;
    for name in module_names {
        let lower = name.to_ascii_lowercase();
        if lower == "gameassembly.dll" {
            return Runtime::Il2cpp;
        }
        if MONO.contains(&lower.as_str()) {
            mono = true;
        }
    }
    if mono {
        Runtime::Mono
    } else {
        Runtime::Native
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_game_is_native() {
        let names = ["ntdll.dll", "witcher2.exe", "d3d11.dll"];
        assert_eq!(of(names), Runtime::Native);
    }

    #[test]
    fn the_mono_runtime_is_spotted_under_any_of_its_names() {
        assert_eq!(of(["game.exe", "mono-2.0-bdwgc.dll"]), Runtime::Mono);
        assert_eq!(of(["game.exe", "MonoBleedingEdge.dll"]), Runtime::Mono);
        assert_eq!(of(["game.exe", "mono.dll"]), Runtime::Mono);
    }

    #[test]
    fn gameassembly_means_il2cpp() {
        assert_eq!(of(["game.exe", "GameAssembly.dll"]), Runtime::Il2cpp);
    }

    // a game that shipped both, il2cpp is the real backend
    #[test]
    fn il2cpp_wins_when_both_look_present() {
        assert_eq!(of(["mono.dll", "GameAssembly.dll"]), Runtime::Il2cpp);
    }

    #[test]
    fn unity_covers_both_managed_backends() {
        assert!(Runtime::Mono.is_unity());
        assert!(Runtime::Il2cpp.is_unity());
        assert!(!Runtime::Native.is_unity());
    }
}
