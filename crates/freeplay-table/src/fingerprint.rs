use crate::schema::{Action, Locator, Table};

// sha256, written out rather than pulled in. it is a page of code and the only
// thing we hash is a few kb of table text
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut message = data.to_vec();
    let bits = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());

    for chunk in message.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }

    h.iter().map(|word| format!("{word:08x}")).collect()
}

// what the table actually does, with the parts that do not change behaviour
// left out. two people converting the same .CT get the same fingerprint even
// if one of them renamed the game or typed a different freeze value, so we can
// tell them we already have it instead of asking them to upload it again
pub fn fingerprint(table: &Table) -> String {
    let mut parts = vec![table.game.exe.to_lowercase()];

    let mut cheats: Vec<String> = table
        .cheats
        .iter()
        .map(|cheat| {
            let what = match &cheat.action {
                Action::Freeze { kind, .. } => format!("freeze:{:?}", kind.0),
                Action::Set { kind, .. } => format!("set:{:?}", kind.0),
                Action::Nop { length } => format!("nop:{length}"),
                Action::Bytes { replacement } => {
                    format!(
                        "bytes:{}",
                        replacement.split_whitespace().collect::<String>()
                    )
                }
                Action::Script { source } => format!("script:{}", squash(source)),
            };
            format!("{}|{what}", locator_of(cheat.locator.as_ref()))
        })
        .collect();

    cheats.sort();
    parts.extend(cheats);
    sha256(parts.join("\n").as_bytes())
}

fn locator_of(locator: Option<&Locator>) -> String {
    match locator {
        None => "none".into(),
        Some(Locator::Static {
            module,
            offset,
            hops,
        }) => format!(
            "static:{}:{offset:x}:{}",
            module.to_lowercase(),
            hops_of(hops)
        ),
        Some(Locator::Symbol { symbol, hops }) => format!("symbol:{symbol}:{}", hops_of(hops)),
        Some(Locator::Pattern {
            pattern,
            module,
            offset,
            hops,
            ..
        }) => format!(
            "pattern:{}:{}:{offset}:{}",
            pattern
                .split_whitespace()
                .collect::<String>()
                .to_lowercase(),
            module.as_deref().unwrap_or("").to_lowercase(),
            hops_of(hops)
        ),
    }
}

fn hops_of(hops: &[crate::schema::Hop]) -> String {
    hops.iter()
        .map(|h| h.0.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

// whitespace and case in assembly do not change what it does
fn squash(source: &str) -> String {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join(";")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = r#"
        [game]
        name = "Some Game"
        exe = "game.exe"

        [[cheat]]
        id = "health"
        name = "Health"
        type = "freeze"
        value_type = "f32"
        value = 1000

        [cheat.locator]
        find = "static"
        module = "game.exe"
        offset = "0x1A2B3C"
        hops = ["+0x28"]
    "#;

    fn table(text: &str) -> Table {
        Table::parse(text).unwrap()
    }

    #[test]
    fn matches_the_known_sha256_of_an_empty_input() {
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn matches_the_known_sha256_of_abc() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashes_more_than_one_block() {
        let long = "a".repeat(1000);
        assert_eq!(sha256(long.as_bytes()).len(), 64);
        assert_ne!(sha256(long.as_bytes()), sha256(b"a"));
    }

    #[test]
    fn the_same_table_twice_is_the_same_fingerprint() {
        assert_eq!(fingerprint(&table(ONE)), fingerprint(&table(ONE)));
    }

    #[test]
    fn renaming_the_game_does_not_change_it() {
        let renamed = ONE.replace("Some Game", "Totally Different Name");
        assert_eq!(fingerprint(&table(ONE)), fingerprint(&table(&renamed)));
    }

    // the value is somebody's preference, not part of what the table found
    #[test]
    fn a_different_freeze_value_does_not_change_it() {
        let bigger = ONE.replace("value = 1000", "value = 99999");
        assert_eq!(fingerprint(&table(ONE)), fingerprint(&table(&bigger)));
    }

    #[test]
    fn renaming_a_cheat_does_not_change_it() {
        let renamed = ONE.replace(r#"name = "Health""#, r#"name = "Vitality""#);
        assert_eq!(fingerprint(&table(ONE)), fingerprint(&table(&renamed)));
    }

    #[test]
    fn a_different_address_does_change_it() {
        let moved = ONE.replace("0x1A2B3C", "0x1A2B40");
        assert_ne!(fingerprint(&table(ONE)), fingerprint(&table(&moved)));
    }

    #[test]
    fn a_different_hop_changes_it() {
        let moved = ONE.replace(r#"hops = ["+0x28"]"#, r#"hops = ["+0x30"]"#);
        assert_ne!(fingerprint(&table(ONE)), fingerprint(&table(&moved)));
    }

    #[test]
    fn a_different_game_changes_it() {
        let other = ONE.replace(r#"exe = "game.exe""#, r#"exe = "other.exe""#);
        assert_ne!(fingerprint(&table(ONE)), fingerprint(&table(&other)));
    }

    #[test]
    fn the_order_cheats_are_written_in_does_not_matter() {
        let two = r#"
            [game]
            name = "G"
            exe = "g.exe"

            [[cheat]]
            id = "a"
            name = "A"
            type = "set"
            value_type = "i32"
            value = 1
            [cheat.locator]
            find = "static"
            module = "g.exe"
            offset = "0x10"

            [[cheat]]
            id = "b"
            name = "B"
            type = "set"
            value_type = "i32"
            value = 2
            [cheat.locator]
            find = "static"
            module = "g.exe"
            offset = "0x20"
        "#;
        let flipped = r#"
            [game]
            name = "G"
            exe = "g.exe"

            [[cheat]]
            id = "b"
            name = "B"
            type = "set"
            value_type = "i32"
            value = 2
            [cheat.locator]
            find = "static"
            module = "g.exe"
            offset = "0x20"

            [[cheat]]
            id = "a"
            name = "A"
            type = "set"
            value_type = "i32"
            value = 1
            [cheat.locator]
            find = "static"
            module = "g.exe"
            offset = "0x10"
        "#;
        assert_eq!(fingerprint(&table(two)), fingerprint(&table(flipped)));
    }

    #[test]
    fn reformatting_a_script_does_not_change_it() {
        let base = r#"
            [game]
            name = "G"
            exe = "g.exe"

            [[cheat]]
            id = "s"
            name = "S"
            type = "script"
            source = """
[ENABLE]
alloc(cave,100)
cave:
  nop
[DISABLE]
dealloc(cave)
"""
        "#;
        let spaced = base.replace("  nop", "      NOP   ");
        assert_eq!(fingerprint(&table(base)), fingerprint(&table(&spaced)));
    }

    #[test]
    fn changing_what_a_script_does_changes_it() {
        let base = r#"
            [game]
            name = "G"
            exe = "g.exe"

            [[cheat]]
            id = "s"
            name = "S"
            type = "script"
            source = """
[ENABLE]
alloc(cave,100)
cave:
  nop
[DISABLE]
dealloc(cave)
"""
        "#;
        let changed = base.replace("  nop", "  ret");
        assert_ne!(fingerprint(&table(base)), fingerprint(&table(&changed)));
    }
}
