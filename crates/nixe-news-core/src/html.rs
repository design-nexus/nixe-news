//! Feed HTML is untrusted. The allowlist is ours, and the reader never gets a
//! script, style, or non-https link.

use ammonia::Builder;
use std::collections::HashSet;

const TAGS: &[&str] = &["p", "br", "h1", "h2", "h3", "ul", "ol", "li", "blockquote", "pre", "code", "strong", "em", "a", "img"];

/// Sanitize a feed fragment. `href` and `src` must be `https`.
pub fn sanitize(input: &str) -> String {
    Builder::default()
        .tags(TAGS.iter().copied().collect::<HashSet<_>>())
        .generic_attributes(HashSet::new())
        .link_rel(None)
        .add_tag_attributes("a", ["href"])
        .add_tag_attributes("img", ["src"])
        .url_schemes(HashSet::from(["https"]))
        .clean(input)
        .to_string()
}

/// Decode HTML character references (`&amp;`, `&nbsp;`, `&#8217;`, `&#x27;`, etc.).
pub fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        if let Some(semi) = after.find(';')
            && semi <= 12
        {
            let entity = &after[..semi];
            if let Some(decoded) = resolve_entity(entity) {
                out.push_str(&decoded);
                rest = &after[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn resolve_entity(entity: &str) -> Option<String> {
    if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
        let code = u32::from_str_radix(hex, 16).ok()?;
        let ch = if code == 0xA0 { ' ' } else { char::from_u32(code)? };
        return Some(ch.to_string());
    }
    if let Some(dec) = entity.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        let ch = if code == 160 { ' ' } else { char::from_u32(code)? };
        return Some(ch.to_string());
    }
    let replacement = match entity {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "ndash" => "–",
        "mdash" => "—",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "hellip" => "…",
        "bull" => "•",
        "middot" => "·",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "euro" => "€",
        "pound" => "£",
        _ => return None,
    };
    Some(replacement.to_string())
}

/// Plain text for the list and for search, capped at `limit` characters.
pub fn excerpt(input: &str, limit: usize) -> String {
    let cleaned = sanitize(input);
    let mut out = String::new();
    let mut in_tag = false;
    for ch in cleaned.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let decoded = decode_entities(&out);
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        collapsed
    } else {
        let mut end = collapsed.char_indices().nth(limit).map(|(i, _)| i).unwrap_or(collapsed.len());
        if let Some(space) = collapsed[..end].rfind(' ')
            && space > end / 2
        {
            end = space;
        }
        format!("{}…", collapsed[..end].trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_and_unsafe_links_are_removed() {
        let html = r#"<p onclick="alert(1)">Hello<script>alert(1)</script></p><a href="javascript:alert(1)">x</a><a href="https://example.com/a">ok</a>"#;
        let clean = sanitize(html);
        assert!(!clean.to_ascii_lowercase().contains("script"));
        assert!(!clean.contains("javascript:"));
        assert!(!clean.contains("onclick"));
        assert!(clean.contains("https://example.com/a"));
        assert!(clean.contains("Hello"));
    }

    #[test]
    fn excerpts_drop_tags() {
        let text = excerpt("<p>Rust <strong>release</strong> notes &amp; updates&#160;2026</p>", 80);
        assert_eq!(text, "Rust release notes & updates 2026");
    }

    #[test]
    fn unicode_surrounding_ampersands_does_not_panic() {
        let text = decode_entities("&don’t; &amp; ‘hello’");
        assert_eq!(text, "&don’t; & ‘hello’");
    }
}
