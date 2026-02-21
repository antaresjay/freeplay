//! Just enough of Valve's key-value format to read a library list.
//!
//! The real format has macros, conditionals and includes. Steam's own
//! libraryfolders.vdf and appmanifest files use none of that, so this handles
//! quoted strings and nested braces and nothing else.

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Map(entries) => entries,
            Vdf::Str(_) => &[],
        }
    }

    pub fn string(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Vdf::as_str)
    }
}

pub fn parse(text: &str) -> Result<Vdf, String> {
    let tokens = tokenize(text)?;
    let mut cursor = 0;
    let entries = parse_entries(&tokens, &mut cursor, true)?;
    Ok(Vdf::Map(entries))
}

#[derive(Debug, PartialEq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => {
                let mut value = String::new();
                loop {
                    match chars.next() {
                        Some('\\') => match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some(other) => value.push(other),
                            None => return Err("string ends in a backslash".into()),
                        },
                        Some('"') => break,
                        Some(other) => value.push(other),
                        None => return Err("unterminated string".into()),
                    }
                }
                tokens.push(Token::Str(value));
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            other => return Err(format!("unexpected character {other:?}")),
        }
    }
    Ok(tokens)
}

fn parse_entries(
    tokens: &[Token],
    cursor: &mut usize,
    top_level: bool,
) -> Result<Vec<(String, Vdf)>, String> {
    let mut entries = Vec::new();

    while *cursor < tokens.len() {
        match &tokens[*cursor] {
            Token::Close => {
                if top_level {
                    return Err("unexpected closing brace".into());
                }
                *cursor += 1;
                return Ok(entries);
            }
            Token::Open => return Err("expected a key".into()),
            Token::Str(key) => {
                *cursor += 1;
                match tokens.get(*cursor) {
                    Some(Token::Str(value)) => {
                        entries.push((key.clone(), Vdf::Str(value.clone())));
                        *cursor += 1;
                    }
                    Some(Token::Open) => {
                        *cursor += 1;
                        let nested = parse_entries(tokens, cursor, false)?;
                        entries.push((key.clone(), Vdf::Map(nested)));
                    }
                    _ => return Err(format!("key {key:?} has no value")),
                }
            }
        }
    }

    if top_level {
        Ok(entries)
    } else {
        Err("missing closing brace".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_flat_map() {
        let v = parse(r#""name" "Mass Effect" "appid" "1328670""#).unwrap();
        assert_eq!(v.string("name"), Some("Mass Effect"));
        assert_eq!(v.string("appid"), Some("1328670"));
    }

    #[test]
    fn parses_nesting() {
        let text = r#"
            "libraryfolders"
            {
                "0"
                {
                    "path"  "C:\\Steam"
                }
                "1"
                {
                    "path"  "D:\\Games"
                }
            }
        "#;
        let v = parse(text).unwrap();
        let folders = v.get("libraryfolders").unwrap();
        assert_eq!(folders.entries().len(), 2);
        assert_eq!(folders.get("0").unwrap().string("path"), Some(r"C:\Steam"));
        assert_eq!(folders.get("1").unwrap().string("path"), Some(r"D:\Games"));
    }

    #[test]
    fn keys_are_case_insensitive() {
        let v = parse(r#""AppID" "42""#).unwrap();
        assert_eq!(v.string("appid"), Some("42"));
    }

    #[test]
    fn skips_line_comments() {
        let v = parse("// a comment\n\"a\" \"b\"").unwrap();
        assert_eq!(v.string("a"), Some("b"));
    }

    #[test]
    fn handles_escaped_quotes() {
        let v = parse(r#""name" "He said \"hi\"""#).unwrap();
        assert_eq!(v.string("name"), Some(r#"He said "hi""#));
    }

    #[test]
    fn rejects_a_key_with_no_value() {
        assert!(parse(r#""lonely""#).is_err());
    }

    #[test]
    fn rejects_unbalanced_braces() {
        assert!(parse(r#""a" { "b" "c" "#).is_err());
        assert!(parse(r#""a" "b" } "#).is_err());
    }

    #[test]
    fn empty_input_is_an_empty_map() {
        assert_eq!(parse("").unwrap(), Vdf::Map(vec![]));
    }
}
