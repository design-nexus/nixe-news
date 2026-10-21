//! HTTP fetch with timeouts, a size cap, and checks against private and
//! loopback targets. Redirects are followed by hand so each hop is checked
//! before a connection.

use anyhow::{Context, Result, anyhow, bail};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::time::Duration;
use url::Url;

pub const MAX_FEED_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct FetchOpts {
    pub allow_http: bool,
    /// Test switch. Production refresh leaves this false.
    pub allow_private: bool,
    /// A feed that redirects to another host is refused unless this is set.
    pub allow_host_change: bool,
    pub max_bytes: usize,
    pub timeout: Duration,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// Replaces the default client name. Story pages set a browser name so the publisher returns the article.
    pub user_agent: Option<String>,
}

impl Default for FetchOpts {
    fn default() -> Self {
        Self {
            allow_http: false,
            allow_private: false,
            allow_host_change: false,
            max_bytes: MAX_FEED_BYTES,
            timeout: Duration::from_secs(20),
            etag: None,
            last_modified: None,
            user_agent: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchBody {
    pub bytes: Vec<u8>,
    pub final_url: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchResult {
    NotModified { final_url: String },
    Body(FetchBody),
}

pub fn validate_url(input: &str, allow_http: bool) -> Result<Url> {
    let url = Url::parse(input.trim()).context("the address could not be parsed")?;
    match url.scheme() {
        "https" => {}
        "http" if allow_http => {}
        "http" => bail!("only https feeds are added, unless you typed an http address"),
        other => bail!("{other} addresses are not fetched"),
    }
    if url.host_str().is_none() {
        bail!("the address has no host");
    }
    if url.username() != "" || url.password().is_some() {
        bail!("addresses with a username or password are not fetched");
    }
    Ok(url)
}

pub fn ip_allowed(ip: IpAddr, allow_private: bool) -> bool {
    if allow_private {
        return !is_documentation(ip) && !ip_unspecified_or_multicast(ip);
    }
    !is_blocked(ip)
}

fn ip_unspecified_or_multicast(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_unspecified() || v.is_multicast() || v.is_broadcast(),
        IpAddr::V6(v) => v.is_unspecified() || v.is_multicast(),
    }
}

fn is_documentation(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_documentation() || v.is_unspecified() || v.is_broadcast() || v.is_multicast(),
        IpAddr::V6(v) => is_v6_documentation(v) || v.is_unspecified() || v.is_multicast(),
    }
}

fn is_v6_documentation(v: Ipv6Addr) -> bool {
    v.segments()[0] == 0x2001 && v.segments()[1] == 0x0db8
}

fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_multicast()
                || v.is_broadcast()
                || v.is_documentation()
                || is_cgnat(v)
                || is_benchmark(v)
        }
        IpAddr::V6(v) => {
            let mapped = v.to_ipv4_mapped();
            v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || is_unique_local(v)
                || is_v6_link_local(v)
                || is_v6_documentation(v)
                || mapped.is_some_and(|v4| is_blocked(IpAddr::V4(v4)))
        }
    }
}

fn is_cgnat(v: Ipv4Addr) -> bool {
    v.octets()[0] == 100 && (v.octets()[1] & 0xc0) == 64
}

fn is_benchmark(v: Ipv4Addr) -> bool {
    v.octets()[0] == 198 && v.octets()[1] == 18
}

fn is_unique_local(v: Ipv6Addr) -> bool {
    (v.segments()[0] & 0xfe00) == 0xfc00
}

fn is_v6_link_local(v: Ipv6Addr) -> bool {
    (v.segments()[0] & 0xffc0) == 0xfe80
}

pub fn validate_resolved(url: &Url, ips: &[IpAddr], allow_private: bool) -> Result<()> {
    if ips.is_empty() {
        bail!("{} did not resolve", url.host_str().unwrap_or("the host"));
    }
    if let Some(ip) = ips.iter().find(|ip| !ip_allowed(**ip, allow_private)) {
        bail!("{ip} is not a public address");
    }
    Ok(())
}

fn resolve(url: &Url) -> Result<Vec<IpAddr>> {
    let host = url.host_str().context("the address has no host")?;
    let port = url.port_or_known_default().unwrap_or(443);
    let mut ips = Vec::new();
    for addr in (host, port).to_socket_addrs().with_context(|| format!("could not resolve {host}"))? {
        ips.push(addr.ip());
    }
    ips.sort();
    ips.dedup();
    Ok(ips)
}

fn header(resp: &reqwest::blocking::Response, name: &str) -> Option<String> {
    resp.headers().get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
}

/// Fetch `input`, following at most five redirects after each hop is checked.
pub fn get(input: &str, opts: &FetchOpts) -> Result<FetchResult> {
    let start = validate_url(input, opts.allow_http || input.trim().starts_with("http://"))?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(opts.timeout)
        .connect_timeout(Duration::from_secs(10))
        .user_agent(opts.user_agent.as_deref().unwrap_or("NixeNews/0.1 (+https://github.com/design-nexus/nixe)"))
        .build()
        .context("could not build the HTTP client")?;
    let mut current = start;
    let origin_host = current.host_str().unwrap_or_default().to_ascii_lowercase();
    for _ in 0..5 {
        let ips = resolve(&current)?;
        validate_resolved(&current, &ips, opts.allow_private)?;
        let mut req = client.get(current.clone());
        if let Some(etag) = &opts.etag {
            req = req.header("if-none-match", etag);
        }
        if let Some(modified) = &opts.last_modified {
            req = req.header("if-modified-since", modified);
        }
        let resp = req.send().with_context(|| format!("could not fetch {current}"))?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = header(&resp, "location").context("a redirect had no location")?;
            let next = current.join(&loc).context("the redirect location was not a URL")?;
            let next_host = next.host_str().unwrap_or_default().to_ascii_lowercase();
            if next_host != origin_host && !opts.allow_host_change {
                bail!("the feed redirected to another host");
            }
            validate_url(next.as_str(), opts.allow_http || next.scheme() == "http")?;
            current = next;
            continue;
        }
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchResult::NotModified { final_url: current.to_string() });
        }
        if !status.is_success() {
            bail!("the server returned {status}");
        }
        if let Some(len) = resp.content_length()
            && len > opts.max_bytes as u64
        {
            bail!("the feed is larger than {} bytes", opts.max_bytes);
        }
        let etag = header(&resp, "etag");
        let last_modified = header(&resp, "last-modified");
        let bytes = read_capped(resp, opts.max_bytes)?;
        return Ok(FetchResult::Body(FetchBody { bytes, final_url: current.to_string(), etag, last_modified }));
    }
    Err(anyhow!("the feed redirected too many times"))
}

fn read_capped(resp: reqwest::blocking::Response, max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    let mut limited = resp.take(max as u64 + 1);
    limited.read_to_end(&mut buf).context("the response stopped while it was being read")?;
    if buf.len() > max {
        bail!("the feed is larger than {max} bytes");
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn private_and_odd_addresses_are_refused() {
        let blocked = [
            "127.0.0.1",
            "10.1.1.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "192.0.2.1",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
        ];
        for text in blocked {
            let ip: IpAddr = text.parse().unwrap();
            assert!(!ip_allowed(ip, false), "{text}");
        }
        assert!(ip_allowed("1.1.1.1".parse().unwrap(), false));
        assert!(ip_allowed("127.0.0.1".parse().unwrap(), true));
        assert!(!ip_allowed("192.0.2.1".parse().unwrap(), true));
    }

    #[test]
    fn schemes_other_than_https_are_rejected() {
        assert!(validate_url("file:///tmp/x", false).is_err());
        assert!(validate_url("javascript:alert(1)", false).is_err());
        assert!(validate_url("http://example.com/feed", false).is_err());
        assert!(validate_url("https://example.com/feed", false).is_ok());
        assert!(validate_url("https://user:pass@example.com/feed", false).is_err());
    }

    #[test]
    fn a_redirect_to_another_host_is_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0; 512];
            let _ = sock.read(&mut buf);
            let body =
                "HTTP/1.1 302 Found\r\nLocation: http://192.0.2.1/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = sock.write_all(body.as_bytes());
        });
        let opts = FetchOpts { allow_http: true, allow_private: true, ..FetchOpts::default() };
        let err = get(&format!("http://127.0.0.1:{port}/feed"), &opts).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("another host") || message.contains("not a public"), "{message}");
    }
}
