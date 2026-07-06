//! deciding which shared table to put first
//!
//! this runs here rather than on the service because the only machine that
//! knows exactly which build of the game is installed is this one. a table
//! somebody ran against your build is worth more than one with twenty votes
//! from two patches ago, and no amount of sorting on the server can know that.

use crate::community::Listing;

// how a table's build lines up with the one installed here
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    // ran against the build sitting on this machine
    Same,
    // ran against an older build of the game
    Older,
    // ran against a newer one, so the game here is behind
    Newer,
    // one side or the other never said
    Unknown,
}

impl Fit {
    pub fn key(self) -> &'static str {
        match self {
            Fit::Same => "same",
            Fit::Older => "older",
            Fit::Newer => "newer",
            Fit::Unknown => "unknown",
        }
    }
}

pub fn fit_of(theirs: &str, mine: &str) -> Fit {
    let (Some(theirs), Some(mine)) = (parts(theirs), parts(mine)) else {
        return Fit::Unknown;
    };
    match theirs.cmp(&mine) {
        std::cmp::Ordering::Equal => Fit::Same,
        std::cmp::Ordering::Less => Fit::Older,
        std::cmp::Ordering::Greater => Fit::Newer,
    }
}

// "3.5.0.1" into something comparable. anything that is not a run of numbers
// separated by dots is not a version we can reason about
fn parts(text: &str) -> Option<Vec<u64>> {
    let text = text.trim().trim_start_matches('v');
    if text.is_empty() {
        return None;
    }
    let found: Option<Vec<u64>> = text.split('.').map(|p| p.trim().parse().ok()).collect();
    found.filter(|v| !v.is_empty())
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub listing: Listing,
    pub fit: Fit,
    pub score: f64,
    // the one to put a star on, at most one in a list
    pub recommended: bool,
}

/// Rank tables for the machine this is running on, best first.
pub fn rank(rows: Vec<Listing>, mine: &str, now: i64) -> Vec<Scored> {
    let mut scored: Vec<Scored> = rows
        .into_iter()
        .map(|listing| {
            let fit = fit_of(&listing.built_for, mine);
            Scored {
                score: score(&listing, fit, now),
                fit,
                listing,
                recommended: false,
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            // two identical scores must not shuffle between refreshes
            .then_with(|| a.listing.id.cmp(&b.listing.id))
    });

    // a star on the top one only, and only when there is a reason for it.
    // starring the single unproven table somebody uploaded this morning would
    // make the word mean nothing
    if let Some(first) = scored.first_mut() {
        first.recommended = worth_recommending(first);
    }
    scored
}

fn worth_recommending(top: &Scored) -> bool {
    if top.fit == Fit::Older || top.fit == Fit::Newer {
        return false;
    }
    let votes = top.listing.up > top.listing.down;
    top.fit == Fit::Same || (votes && top.listing.downloads > 0)
}

fn score(row: &Listing, fit: Fit, now: i64) -> f64 {
    let mut score = match fit {
        Fit::Same => 100.0,
        Fit::Unknown => 0.0,
        // a table for another build usually resolves nothing at all, which is
        // worse than one that simply never said which build it was for
        Fit::Older | Fit::Newer => -45.0,
    };

    // the share of people who said it worked, discounted for how few of them
    // there were, so two out of two does not beat ninety out of a hundred.
    // centred on a half, because a table forty people say is broken has to be
    // pushed down, not merely left unrewarded
    if row.up + row.down > 0 {
        score += (confidence(row.up, row.down) - 0.5) * 60.0;
    }

    // downloads say people found it, not that it worked. a popular table
    // everybody downvoted is a popular broken table
    score += (1.0 + row.downloads.max(0) as f64).ln() * 3.0;

    // a table published after the last patch is more likely to survive it
    let days = ((now - row.created_at).max(0) as f64) / 86_400.0;
    score += (12.0 - days / 30.0).clamp(0.0, 12.0);

    // a tiebreak, nothing more. more cheats is not better, it is just more
    score += (row.cheats.min(40) as f64) * 0.05;
    score
}

// lower bound of the wilson interval, the usual answer to "rank these by
// rating when some have three votes and some have three hundred"
fn confidence(up: i64, down: i64) -> f64 {
    let n = (up.max(0) + down.max(0)) as f64;
    if n == 0.0 {
        return 0.0;
    }
    let p = up.max(0) as f64 / n;
    let z = 1.96;
    let z2 = z * z;

    let centre = p + z2 / (2.0 * n);
    let spread = z * ((p * (1.0 - p) + z2 / (4.0 * n)) / n).sqrt();
    ((centre - spread) / (1.0 + z2 / n)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn table(id: i64, built_for: &str, up: i64, down: i64, downloads: i64) -> Listing {
        Listing {
            id,
            built_for: built_for.into(),
            up,
            down,
            downloads,
            created_at: NOW - 86_400 * 30,
            cheats: 20,
            ..Default::default()
        }
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert_eq!(fit_of("3.5.0.1", "3.5.0.1"), Fit::Same);
        assert_eq!(fit_of("3.4.0.2", "3.5.0.1"), Fit::Older);
        assert_eq!(fit_of("3.6", "3.5.0.1"), Fit::Newer);
        // the text comparison everybody writes by accident says 10 < 9
        assert_eq!(fit_of("1.10.0", "1.9.0"), Fit::Newer);
    }

    #[test]
    fn a_version_nobody_wrote_down_is_unknown_not_wrong() {
        assert_eq!(fit_of("", "3.5.0.1"), Fit::Unknown);
        assert_eq!(fit_of("3.5.0.1", ""), Fit::Unknown);
        assert_eq!(fit_of("release candidate", "3.5.0.1"), Fit::Unknown);
    }

    #[test]
    fn a_leading_v_is_not_a_different_version() {
        assert_eq!(fit_of("v3.5.0.1", "3.5.0.1"), Fit::Same);
    }

    // the whole point. a table checked against the build you are running beats
    // a popular one that was checked against a different build
    #[test]
    fn your_build_beats_a_pile_of_votes_from_another_one() {
        let ranked = rank(
            vec![
                table(1, "3.4.0.2", 90, 2, 5000),
                table(2, "3.5.0.1", 1, 0, 3),
            ],
            "3.5.0.1",
            NOW,
        );
        assert_eq!(ranked[0].listing.id, 2);
        assert_eq!(ranked[0].fit, Fit::Same);
        assert!(ranked[0].recommended);
    }

    #[test]
    fn a_table_for_your_build_beats_one_that_never_said() {
        let ranked = rank(
            vec![table(1, "", 5, 0, 100), table(2, "3.5.0.1", 0, 0, 0)],
            "3.5.0.1",
            NOW,
        );
        assert_eq!(ranked[0].listing.id, 2);
    }

    #[test]
    fn a_few_good_votes_do_not_beat_many_good_votes() {
        let ranked = rank(
            vec![table(1, "", 2, 0, 10), table(2, "", 180, 20, 10)],
            "3.5.0.1",
            NOW,
        );
        assert_eq!(ranked[0].listing.id, 2, "two out of two is not proof");
    }

    #[test]
    fn downvotes_sink_it() {
        let ranked = rank(
            vec![
                table(1, "3.5.0.1", 2, 40, 900),
                table(2, "3.5.0.1", 8, 1, 5),
            ],
            "3.5.0.1",
            NOW,
        );
        assert_eq!(ranked[0].listing.id, 2);
    }

    #[test]
    fn nothing_is_recommended_when_the_best_on_offer_is_for_another_build() {
        let ranked = rank(vec![table(1, "3.4.0.2", 90, 0, 5000)], "3.5.0.1", NOW);
        assert_eq!(ranked[0].fit, Fit::Older);
        assert!(
            !ranked[0].recommended,
            "do not point people at a broken one"
        );
    }

    #[test]
    fn an_unproven_upload_is_listed_but_not_starred() {
        let ranked = rank(vec![table(1, "", 0, 0, 0)], "3.5.0.1", NOW);
        assert_eq!(ranked.len(), 1);
        assert!(!ranked[0].recommended);
    }

    #[test]
    fn only_ever_one_star() {
        let ranked = rank(
            vec![
                table(1, "3.5.0.1", 10, 0, 100),
                table(2, "3.5.0.1", 9, 0, 90),
                table(3, "3.5.0.1", 8, 0, 80),
            ],
            "3.5.0.1",
            NOW,
        );
        assert_eq!(ranked.iter().filter(|r| r.recommended).count(), 1);
    }

    #[test]
    fn the_order_does_not_wander_between_refreshes() {
        let rows = vec![table(7, "", 0, 0, 0), table(3, "", 0, 0, 0)];
        let once = rank(rows.clone(), "3.5.0.1", NOW);
        let twice = rank(rows, "3.5.0.1", NOW);
        let ids: Vec<i64> = once.iter().map(|r| r.listing.id).collect();
        let again: Vec<i64> = twice.iter().map(|r| r.listing.id).collect();
        assert_eq!(ids, again);
        assert_eq!(ids, vec![3, 7], "identical scores fall back to the id");
    }

    #[test]
    fn something_published_after_the_last_patch_edges_ahead() {
        let mut fresh = table(1, "", 3, 0, 50);
        fresh.created_at = NOW - 86_400;
        let mut old = table(2, "", 3, 0, 50);
        old.created_at = NOW - 86_400 * 800;

        let ranked = rank(vec![old, fresh], "3.5.0.1", NOW);
        assert_eq!(ranked[0].listing.id, 1);
    }

    #[test]
    fn confidence_is_between_nothing_and_everything() {
        assert_eq!(confidence(0, 0), 0.0);
        assert!(confidence(1, 0) > 0.0 && confidence(1, 0) < 1.0);
        assert!(confidence(1000, 0) > confidence(10, 0));
        assert_eq!(confidence(0, 50), 0.0);
    }

    #[test]
    fn an_empty_list_is_an_empty_list() {
        assert!(rank(Vec::new(), "3.5.0.1", NOW).is_empty());
    }
}
