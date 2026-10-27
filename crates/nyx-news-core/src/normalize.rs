//! Canonical article and feed URLs. The original URL is kept for the source link.

use anyhow::{Result, bail};
use url::Url;

const TRACKING: &[&str] = &[
    "fbclid", "mc_cid", "mc_eid", "gclid", "igshid", "msclkid", "dclid",
    "traffic_source", "ref", "source",
];

/// `http` and `https` only. Lowercase host, default port removed, fragment
/// removed, and a fixed set of tracking parameters removed.
pub fn canonical_url(input: &str) -> Result<String> {
    let url = Url::parse(input.trim()).or_else(|_| Url::parse(&format!("https://{input}")))?;
    canonicalize(url)
}

pub fn canonicalize(mut url: Url) -> Result<String> {
    match url.scheme() {
        "https" | "http" => {}
        other => bail!("unsupported URL scheme {other}"),
    }
    if url.host_str().is_none() {
        bail!("the URL has no host");
    }
    url.set_fragment(None);
    if url.port() == Some(url.port_or_known_default().unwrap_or(0)) {
        let _ = url.set_port(None);
    }
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| {
            let key = k.to_ascii_lowercase();
            !key.starts_with("utm_") && !key.starts_with("at_") && !TRACKING.contains(&key.as_str())
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(&kept);
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() {
        bail!("the URL has no host");
    }
    url.set_host(Some(&host))?;
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_parameters_and_fragments_are_removed() {
        let url = canonical_url("HTTPS://Example.COM:443/a/b?utm_source=nl&id=1&fbclid=x#section").unwrap();
        assert_eq!(url, "https://example.com/a/b?id=1");
    }

    #[test]
    fn file_urls_are_rejected() {
        assert!(canonical_url("file:///etc/passwd").is_err());
    }
}
