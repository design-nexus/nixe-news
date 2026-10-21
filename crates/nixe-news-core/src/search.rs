//! Build an FTS5 MATCH expression from typed text. User text is quoted, never
//! concatenated into SQL.

use crate::model::View;

/// Quote each word and join them with AND. Empty input matches nothing useful,
/// so the caller should skip FTS.
pub fn fts_match(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    for raw in input.split_whitespace() {
        let word: String = raw.chars().filter(|c| !matches!(c, '"' | '*' | ':' | '(' | ')' | '^')).collect();
        if word.is_empty() {
            continue;
        }
        terms.push(format!("\"{}\"", word.replace('"', "\"\"")));
    }
    if terms.is_empty() { None } else { Some(terms.join(" AND ")) }
}

pub struct LibraryQuery {
    pub view: View,
    pub text: String,
}

/// Latest articles, unread, saved, or free text. `news`, `news unread`, and `news saved`
/// are the commands. Anything else is a search.
pub fn library_query(raw: &str) -> LibraryQuery {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.is_empty() || lower == "news" {
        return LibraryQuery { view: View::All, text: String::new() };
    }
    for (prefix, view) in [("news unread", View::Unread), ("news saved", View::Saved)] {
        if lower == prefix {
            return LibraryQuery { view, text: String::new() };
        }
        let with_space = format!("{prefix} ");
        if lower.starts_with(&with_space) {
            return LibraryQuery { view, text: trimmed[with_space.len()..].trim().to_string() };
        }
    }
    LibraryQuery { view: View::All, text: trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_commands_pick_a_list_and_leave_the_rest_as_text() {
        let latest = library_query("news");
        assert!(matches!(latest.view, View::All));
        assert!(latest.text.is_empty());
        let unread = library_query("news unread rust");
        assert!(matches!(unread.view, View::Unread));
        assert_eq!(unread.text, "rust");
        let saved = library_query("News Saved");
        assert!(matches!(saved.view, View::Saved));
        assert!(saved.text.is_empty());
        let free = library_query("kernel");
        assert!(matches!(free.view, View::All));
        assert_eq!(free.text, "kernel");
    }

    #[test]
    fn words_are_quoted_and_operators_are_not_passed_through() {
        assert_eq!(fts_match("  rust OR release ").as_deref(), Some("\"rust\" AND \"OR\" AND \"release\""));
        assert_eq!(fts_match("   ").as_deref(), None);
        assert_eq!(fts_match("a\"b").as_deref(), Some("\"ab\""));
    }
}
