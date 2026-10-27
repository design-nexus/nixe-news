//! Group articles that share a title inside a time window, and score a local
//! recommendation from tags, stars, and folder names. No model.

use std::collections::BTreeSet;

pub const WINDOW_SECS: i64 = 48 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    pub weight: i32,
    pub why: String,
    pub tokens: BTreeSet<String>,
}

pub fn title_key(title: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in title.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            gap = false;
        } else if !gap && !out.is_empty() {
            out.push(' ');
            gap = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

pub fn tokens(text: &str) -> BTreeSet<String> {
    title_key(text).split_whitespace().filter(|word| word.len() > 2).map(str::to_string).collect()
}

/// Highest-weight matching signal, plus the sum of every hit.
pub fn score(signals: &[Signal], title: &str, summary: &str, feed: &str, folder: Option<&str>) -> Option<(i32, String)> {
    let mut bag = tokens(title);
    bag.extend(tokens(summary));
    bag.extend(tokens(feed));
    if let Some(folder) = folder {
        bag.extend(tokens(folder));
    }
    let mut total = 0;
    let mut best: Option<(i32, String)> = None;
    for signal in signals {
        if signal.tokens.is_empty() || !signal.tokens.iter().any(|token| bag.contains(token)) {
            continue;
        }
        total += signal.weight;
        let replace = best.as_ref().is_none_or(|(weight, _)| signal.weight > *weight);
        if replace {
            best = Some((signal.weight, signal.why.clone()));
        }
    }
    best.map(|(_, why)| (total, why))
}

pub fn same_story(left_title: &str, left_at: Option<i64>, right_title: &str, right_at: Option<i64>) -> bool {
    let key = title_key(left_title);
    if key.is_empty() || key != title_key(right_title) {
        return false;
    }
    match (left_at, right_at) {
        (Some(left), Some(right)) => (left - right).abs() <= WINDOW_SECS,
        _ => true,
    }
}

/// `items` are `(score, index)` already sorted best-first. Returns groups of indexes.
pub fn group_sorted(
    items: &[(i32, usize)],
    title: impl Fn(usize) -> String,
    when: impl Fn(usize) -> Option<i64>,
) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (_, index) in items {
        let placed = groups.iter_mut().find(|group| {
            let head = group[0];
            same_story(&title(*index), when(*index), &title(head), when(head))
        });
        if let Some(group) = placed {
            group.push(*index);
        } else {
            groups.push(vec![*index]);
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_starred_feed_explains_the_suggestion() {
        let signals = vec![Signal { weight: 4, why: "You starred Ars Technica".into(), tokens: tokens("Ars Technica") }];
        let (score, why) = score(&signals, "A new chip", "", "Ars Technica", None).unwrap();
        assert!(score >= 4);
        assert_eq!(why, "You starred Ars Technica");
    }

    #[test]
    fn an_unrelated_article_is_not_recommended() {
        let signals = vec![Signal { weight: 4, why: "You starred Ars Technica".into(), tokens: tokens("Ars Technica") }];
        assert!(score(&signals, "City council notes", "roads", "Local paper", None).is_none());
    }

    #[test]
    fn duplicates_inside_the_window_collapse() {
        let titles = ["Ship lands", "Ship lands!", "Ship lands"];
        let times = [1_000_i64, 1_000 + 3600, 1_000 + 3600 + WINDOW_SECS + 10];
        let order = [(2, 1usize), (1, 0), (1, 2)];
        let groups = group_sorted(&order, |index| titles[index].into(), |index| Some(times[index]));
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0], vec![1, 0]);
        assert_eq!(groups[1], vec![2]);
    }
}
