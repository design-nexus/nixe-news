//! Turn sanitized feed HTML into plain text for the GTK reader.

use nixe_news_core::decode_entities;

pub fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            tag.clear();
            continue;
        }
        if ch == '>' && in_tag {
            in_tag = false;
            let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("");
            if matches!(name, "p" | "br" | "h1" | "h2" | "h3" | "li" | "blockquote") && !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        if in_tag {
            tag.push(ch);
        } else {
            out.push(ch);
        }
    }
    let decoded = decode_entities(&out);
    decoded
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_and_entities_stay_intact() {
        let html = "<p>First &amp; foremost.</p><p>Second&#160;paragraph.</p>";
        assert_eq!(plain(html), "First & foremost.\n\nSecond paragraph.");
    }
}
