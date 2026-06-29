// 256 short words, one byte each. picked to be easy to write down and hard to
// mistake for each other when read back off a bit of paper
pub const WORDS: &str = "
able acid acorn actor adapt add adopt adult after again agent agree ahead
aim air alarm album alert alien alley alloy alone alpha also amber amend
among amuse anchor angel anger angle ankle apart apple apron arbor arch
arena argue arm armor army aroma array arrow art ash aside ask asset atlas
atom attic audio audit auto avoid awake award aware away axis bacon badge
bag bake balm band bank bar barn base basil basin batch beach beam bean bear
beast begin bell belt bench berry best bike bill bird black blade blend
bless blind blink bliss block blood bloom blue blur board boat body bold
bolt bone bonus book boost boot borrow both bowl box brace brain brand brass
brave bread break brick bridge brief bright bring brisk broad bronze brown
brush bubble bucket budget buffer build bulb bulk bull bunch bundle burst
bus cabin cable cactus cage cake calm camel camp canal candy canvas canyon
cape card cargo carve case cash cast cave cedar cell census chain chair
chalk charm chart chase cheap check cheese chef cherry chess chest chief
child chill chip choice chop chorus cider cinema circle city civil claim
clamp clash clay clean clear clerk click cliff climb clock close cloth cloud
clover club coach coal coast coat cobra code coffee coin cold collar colony
color column comb comet comfort comic coral core cork corn cost cotton couch
cough county cousin cover coyote cozy crab craft crane crash crate crawl
cream credit creek crew crisp crop cross crowd crown cruise crumb
";

pub fn list() -> Vec<&'static str> {
    WORDS.split_whitespace().collect()
}

pub fn index_of(word: &str) -> Option<u8> {
    let lower = word.trim().to_lowercase();
    list()
        .iter()
        .position(|w| *w == lower)
        .map(|position| position as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn there_are_exactly_two_hundred_and_fifty_six() {
        assert_eq!(list().len(), 256, "one byte per word, so it has to be 256");
    }

    #[test]
    fn none_of_them_repeat() {
        let all = list();
        let unique: HashSet<&&str> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "a repeat makes a phrase ambiguous");
    }

    #[test]
    fn they_are_all_plain_lowercase_letters() {
        for word in list() {
            assert!(
                word.chars().all(|c| c.is_ascii_lowercase()),
                "{word} has something odd in it"
            );
            assert!((2..=7).contains(&word.len()), "{word} is an awkward length");
        }
    }

    #[test]
    fn a_word_maps_back_to_its_place() {
        let all = list();
        assert_eq!(index_of(all[0]), Some(0));
        assert_eq!(index_of(all[255]), Some(255));
        assert_eq!(index_of("  ABLE  "), Some(0));
        assert_eq!(index_of("notaword"), None);
    }
}
