//! A Markdown note for one article. This is a file the user saves. It is not written
//! into the Assistant notes database.

/// Title, feed, author, source URL, and the plain article text.
pub fn article_markdown(title: &str, feed: &str, author: &str, url: &str, body: &str) -> String {
    let heading = title.replace(['\n', '\r'], " ");
    let mut out = format!("# {heading}\n\n");
    if !feed.is_empty() {
        out.push_str(feed);
        out.push_str("\n\n");
    }
    if !author.is_empty() {
        out.push_str(author);
        out.push_str("\n\n");
    }
    if !url.is_empty() {
        out.push_str("Source: ");
        out.push_str(url);
        out.push_str("\n\n");
    }
    let body = body.trim();
    if !body.is_empty() {
        out.push_str(body);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_names_the_source_and_keeps_the_text() {
        let note = article_markdown("A title", "Example", "Ada", "https://example.com/a", "The short text.");
        assert!(note.starts_with("# A title\n"));
        assert!(note.contains("Example\n"));
        assert!(note.contains("Source: https://example.com/a\n"));
        assert!(note.contains("The short text.\n"));
    }
}
