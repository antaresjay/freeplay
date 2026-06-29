//! who published a table, without an account you can lose
//!
//! a name is registered against a public key the first time it is used. after
//! that only whoever holds the private key can publish under it, so nobody can
//! take somebody else's name by typing it.
//!
//! there is no password, so there is nothing to reset and nothing on a server
//! worth stealing. the key comes from a phrase you write down once, which is
//! also how it moves to another machine.

pub mod words;

use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// 256 words is one byte each, so 16 of them carry 128 bits and the last is a
// checksum. 128 because that is what ed25519 itself is worth, and there is no
// sense making the phrase the weakest part of the chain
pub const WORD_COUNT: usize = 17;
const ENTROPY: usize = 16;
pub const BITS: usize = ENTROPY * 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phrase(Vec<String>);

impl Phrase {
    pub fn words(&self) -> &[String] {
        &self.0
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let words: Vec<String> = text
            .split_whitespace()
            .map(|w| w.trim().to_lowercase())
            .collect();

        if words.len() != WORD_COUNT {
            return Err(format!(
                "a recovery phrase is {WORD_COUNT} words, that one is {}",
                words.len()
            ));
        }

        for word in &words {
            if words::index_of(word).is_none() {
                return Err(format!("{word} is not one of the words"));
            }
        }

        let phrase = Phrase(words);
        if phrase.bytes()[ENTROPY] != check(&phrase.bytes()[..ENTROPY]) {
            return Err("that phrase has a word wrong in it somewhere".into());
        }
        Ok(phrase)
    }

    fn bytes(&self) -> Vec<u8> {
        self.0.iter().filter_map(|w| words::index_of(w)).collect()
    }

    fn seed(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"freeplay identity v1");
        hasher.update(&self.bytes()[..ENTROPY]);
        hasher.finalize().into()
    }
}

impl std::fmt::Display for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join(" "))
    }
}

fn check(entropy: &[u8]) -> u8 {
    let mut hasher = Sha256::new();
    hasher.update(b"freeplay check v1");
    hasher.update(entropy);
    hasher.finalize()[0]
}

fn from_entropy(entropy: [u8; ENTROPY]) -> Phrase {
    let list = words::list();
    let mut out: Vec<String> = entropy
        .iter()
        .map(|b| list[*b as usize].to_string())
        .collect();
    out.push(list[check(&entropy) as usize].to_string());
    Phrase(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    name: String,
    phrase: String,
}

pub struct Identity {
    pub name: String,
    phrase: Phrase,
    key: SigningKey,
}

impl Identity {
    pub fn create(name: &str) -> Result<Self, String> {
        let mut entropy = [0u8; ENTROPY];
        getrandom(&mut entropy)?;
        Self::from_phrase(name, from_entropy(entropy))
    }

    pub fn from_phrase(name: &str, phrase: Phrase) -> Result<Self, String> {
        let name = name.trim().to_string();
        check_name(&name)?;
        Ok(Self {
            name,
            key: SigningKey::from_bytes(&phrase.seed()),
            phrase,
        })
    }

    pub fn recover(name: &str, text: &str) -> Result<Self, String> {
        Self::from_phrase(name, Phrase::parse(text)?)
    }

    pub fn phrase(&self) -> &Phrase {
        &self.phrase
    }

    pub fn public(&self) -> String {
        hex(self.key.verifying_key().as_bytes())
    }

    pub fn sign(&self, message: &str) -> String {
        hex(&self.key.sign(message.as_bytes()).to_bytes())
    }

    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        let stored: Stored =
            serde_json::from_str(&text).map_err(|e| format!("your identity file is odd: {e}"))?;
        Self::recover(&stored.name, &stored.phrase).map(Some)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let stored = Stored {
            name: self.name.clone(),
            phrase: self.phrase.to_string(),
        };
        let text = serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

// what gets signed. the name stops somebody lifting a signature off one
// submission and reusing it under a different one
pub fn message(name: &str, fingerprint: &str) -> String {
    format!("freeplay/1\n{}\n{fingerprint}", name.trim().to_lowercase())
}

pub fn verify(public: &str, message: &str, signature: &str) -> bool {
    let (Some(public), Some(signature)) = (unhex(public), unhex(signature)) else {
        return false;
    };
    let (Ok(public), Ok(signature)) = (
        <[u8; 32]>::try_from(public.as_slice()),
        <[u8; 64]>::try_from(signature.as_slice()),
    ) else {
        return false;
    };

    let Ok(key) = VerifyingKey::from_bytes(&public) else {
        return false;
    };
    key.verify(message.as_bytes(), &Signature::from_bytes(&signature))
        .is_ok()
}

pub fn check_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.len() < 2 {
        return Err("a name needs at least two characters".into());
    }
    if trimmed.len() > 32 {
        return Err("that name is too long, keep it under 32".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err("letters, numbers, dot, dash and underscore only".into());
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

fn getrandom(into: &mut [u8]) -> Result<(), String> {
    use rand_core::RngCore;
    rand_core::OsRng
        .try_fill_bytes(into)
        .map_err(|e| format!("no randomness available: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn someone() -> Identity {
        Identity::create("aSwedishMagyar").unwrap()
    }

    #[test]
    fn a_new_identity_has_a_phrase_of_the_right_length() {
        let me = someone();
        assert_eq!(me.phrase().words().len(), WORD_COUNT);
        assert_eq!(me.public().len(), 64);
    }

    #[test]
    fn the_same_phrase_gives_back_the_same_key() {
        let me = someone();
        let again = Identity::recover(&me.name, &me.phrase().to_string()).unwrap();
        assert_eq!(me.public(), again.public());
    }

    #[test]
    fn two_identities_are_not_the_same() {
        assert_ne!(someone().public(), someone().public());
    }

    #[test]
    fn a_signature_checks_out() {
        let me = someone();
        let text = message(&me.name, "abc123");
        assert!(verify(&me.public(), &text, &me.sign(&text)));
    }

    #[test]
    fn somebody_elses_key_does_not_check_out() {
        let me = someone();
        let them = someone();
        let text = message(&me.name, "abc123");
        assert!(!verify(&them.public(), &text, &me.sign(&text)));
    }

    // the whole point: typing the name is not enough to publish under it
    #[test]
    fn a_signature_does_not_carry_over_to_another_table() {
        let me = someone();
        let signature = me.sign(&message(&me.name, "abc123"));
        assert!(!verify(
            &me.public(),
            &message(&me.name, "different"),
            &signature
        ));
    }

    #[test]
    fn a_signature_does_not_carry_over_to_another_name() {
        let me = someone();
        let signature = me.sign(&message(&me.name, "abc123"));
        assert!(!verify(
            &me.public(),
            &message("someoneelse", "abc123"),
            &signature
        ));
    }

    #[test]
    fn a_phrase_with_a_typo_is_refused() {
        let me = someone();
        let mut words: Vec<String> = me.phrase().words().to_vec();
        words[0] = "zebra".into();
        let broken = words.join(" ");
        assert!(Phrase::parse(&broken)
            .unwrap_err()
            .contains("not one of the words"));
    }

    // swapping two words keeps every word valid, so only the checksum catches it
    #[test]
    fn a_phrase_with_words_swapped_is_refused() {
        let me = someone();
        let mut words: Vec<String> = me.phrase().words().to_vec();
        if words[0] == words[1] {
            return;
        }
        words.swap(0, 1);
        let outcome = Phrase::parse(&words.join(" "));
        assert!(outcome.is_err(), "the checksum should have caught that");
    }

    #[test]
    fn a_phrase_of_the_wrong_length_is_refused() {
        let why = Phrase::parse("able acid acorn").unwrap_err();
        assert!(why.contains("17 words"), "{why}");
    }

    // the list being public is not the secret, the entropy is. one byte per
    // word and sixteen of them is 2^128, which is what ed25519 is worth
    #[test]
    fn a_phrase_carries_as_many_bits_as_the_curve_is_worth() {
        assert_eq!(words::list().len(), 256);
        assert_eq!(BITS, 128);

        let combinations = (words::list().len() as f64).powi(ENTROPY as i32);
        assert!((combinations.log2() - 128.0).abs() < 0.001);
    }

    #[test]
    fn case_and_spacing_in_a_phrase_do_not_matter() {
        let me = someone();
        let shouted = me.phrase().to_string().to_uppercase().replace(' ', "   ");
        let again = Identity::recover(&me.name, &shouted).unwrap();
        assert_eq!(me.public(), again.public());
    }

    #[test]
    fn names_have_to_be_sensible() {
        assert!(check_name("a").is_err());
        assert!(check_name(&"x".repeat(40)).is_err());
        assert!(check_name("has spaces").is_err());
        assert!(check_name("<script>").is_err());
        assert!(check_name("aSwedishMagyar").is_ok());
        assert!(check_name("some_one-2.0").is_ok());
    }

    #[test]
    fn rubbish_signatures_are_refused_rather_than_panicking() {
        let me = someone();
        let text = message(&me.name, "abc");
        assert!(!verify("not hex", &text, &me.sign(&text)));
        assert!(!verify(&me.public(), &text, "abcd"));
        assert!(!verify("", &text, ""));
        assert!(!verify(&"f".repeat(64), &text, &"f".repeat(128)));
    }

    #[test]
    fn an_identity_survives_a_round_trip_through_a_file() {
        let me = someone();
        let path = std::env::temp_dir().join(format!("freeplay-id-{}.json", std::process::id()));
        me.save(&path).unwrap();

        let back = Identity::load(&path).unwrap().unwrap();
        assert_eq!(back.name, me.name);
        assert_eq!(back.public(), me.public());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_identity_file_is_not_an_error() {
        let path = std::env::temp_dir().join("freeplay-id-definitely-not-here.json");
        assert!(Identity::load(&path).unwrap().is_none());
    }
}
