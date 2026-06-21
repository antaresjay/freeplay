const EDITION_NOISE: &[&str] = &[
    "game of the year edition",
    "definitive edition",
    "enhanced edition",
    "complete edition",
    "ultimate edition",
    "deluxe edition",
    "special edition",
    "director's cut",
    "anniversary edition",
    "remastered",
    "goty edition",
    "goty",
];

fn tidy(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '™' | '®' | '©' => continue,
            c if c.is_alphanumeric() || c == '\'' => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn terms(name: &str) -> String {
    let tidied = tidy(name);
    let lower = tidied.to_lowercase();

    let mut best = tidied.as_str();
    for noise in EDITION_NOISE {
        if let Some(at) = lower.rfind(noise) {
            if at + noise.len() == lower.len() && at > 0 {
                best = tidied[..at].trim_end();
                break;
            }
        }
    }

    if best.is_empty() {
        tidied
    } else {
        best.to_string()
    }
}

pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn search_url(name: &str) -> String {
    let query = format!("site:fearlessrevolution.com {} trainer", terms(name));
    format!("https://duckduckgo.com/?q={}", encode(&query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_an_edition_suffix() {
        assert_eq!(
            terms("The Witcher 2: Assassins of Kings Enhanced Edition"),
            "The Witcher 2 Assassins of Kings"
        );
    }

    #[test]
    fn strips_trademark_marks_stores_put_in_names() {
        assert_eq!(terms("DARK SOULS™ III"), "DARK SOULS III");
    }

    #[test]
    fn punctuation_collapses_rather_than_doubling_up() {
        let url = search_url("Deus Ex: Human Revolution");
        assert!(!url.contains("++"), "{url}");
    }

    #[test]
    fn a_name_that_is_only_an_edition_is_left_alone() {
        assert_eq!(terms("Remastered"), "Remastered");
    }

    #[test]
    fn edition_words_in_the_middle_stay() {
        assert_eq!(
            terms("Special Edition Racing 2"),
            "Special Edition Racing 2"
        );
    }

    #[test]
    fn encodes_what_a_url_cannot_carry() {
        assert_eq!(encode("a b&c"), "a+b%26c");
        assert_eq!(encode("site:example.com"), "site%3Aexample.com");
    }

    #[test]
    fn keeps_an_apostrophe_because_titles_have_them() {
        assert_eq!(terms("Assassin's Creed II"), "Assassin's Creed II");
    }

    #[test]
    fn builds_a_site_scoped_search() {
        let url = search_url("The Witcher 2: Assassins of Kings Enhanced Edition");
        assert!(url.starts_with("https://duckduckgo.com/?q="));
        assert!(url.contains("site%3Afearlessrevolution.com"));
        assert!(url.contains("Witcher"));
        assert!(!url.contains("Enhanced"), "{url}");
    }
}
