//! URL reachability verification.
//!
//! All outbound network I/O for URL checking lives here — the single
//! auditable boundary for external connections made during `skillet build`.
//!
//! # Security controls
//!
//! - Strict URL parser: only `http://` and `https://` schemes accepted; hex,
//!   octal, and integer IP representations are rejected.
//! - DNS is resolved before connecting; all resolved IPs are checked against
//!   a blocklist (RFC 1918, link-local, loopback, IPv6-private, IPv4-mapped).
//! - No redirects are followed (3xx is classified as `ok`).
//! - Only the HTTP status line is read (max 128 bytes); headers and body are
//!   discarded.
//! - 5-second hard wall-clock timeout per connection.
//! - At most 5 simultaneous checks (semaphore-based concurrency cap).
//! - TLS certificate verification is always enforced; there is no option to
//!   disable it.
//! - Uses raw `TcpStream` + `rustls`; no higher-level HTTP library.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT: usize = 5;
/// Maximum bytes read from the response before we stop (status line only).
const STATUS_LINE_MAX: usize = 128;

// ── Public types ─────────────────────────────────────────────────────────────

/// Classification of a URL check attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlCheckResult {
    /// 2xx, 3xx, 401, 403 — URL exists and is reachable.
    Ok,
    /// 404 or 410 — URL definitively missing.
    Broken(u16),
    /// 5xx — server error; may be transient.
    PossiblyDown(u16),
    /// DNS failure, connection refused, timeout, or other network error.
    Unreachable(String),
    /// URL rejected by security policy (non-http(s) scheme, private IP, etc.).
    Rejected(String),
}

/// The outcome of checking one URL.
#[derive(Debug, Clone)]
pub struct UrlCheckOutcome {
    /// The URL that was checked.
    pub url: String,
    /// The result of the check.
    pub result: UrlCheckResult,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Verifies a slice of URLs, returning one outcome per unique URL.
///
/// Checks run concurrently with a cap of [`MAX_CONCURRENT`] simultaneous
/// connections.  Duplicate URLs are deduplicated before dispatching.  Each
/// check has a [`TIMEOUT`] hard wall-clock timeout.
pub fn verify_urls(urls: &[String]) -> Vec<UrlCheckOutcome> {
    // Deduplicate while preserving first-seen order.
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = urls
        .iter()
        .filter(|u| seen.insert(u.as_str().to_string()))
        .cloned()
        .collect();

    if unique.is_empty() {
        return Vec::new();
    }

    // Simple counting semaphore: limits concurrent threads.
    let sem: Arc<(Mutex<usize>, Condvar)> =
        Arc::new((Mutex::new(MAX_CONCURRENT), Condvar::new()));
    let results: Arc<Mutex<Vec<UrlCheckOutcome>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::with_capacity(unique.len());

    for url in unique {
        // Acquire a permit before spawning (blocks if at MAX_CONCURRENT).
        {
            let (lock, cvar) = &*sem;
            let mut permits = lock.lock().expect("semaphore lock poisoned");
            while *permits == 0 {
                permits = cvar.wait(permits).expect("semaphore condvar poisoned");
            }
            *permits -= 1;
        }

        let sem2 = Arc::clone(&sem);
        let res2 = Arc::clone(&results);
        let url2 = url.clone();

        handles.push(thread::spawn(move || {
            let result = check_url(&url2);
            res2.lock()
                .expect("results lock poisoned")
                .push(UrlCheckOutcome { url: url2, result });

            // Release the permit.
            let (lock, cvar) = &*sem2;
            let mut permits = lock.lock().expect("semaphore lock poisoned");
            *permits += 1;
            cvar.notify_one();
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    Arc::try_unwrap(results)
        .expect("all worker threads have finished")
        .into_inner()
        .expect("results lock poisoned")
}

// ── Internal URL parsing ──────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq)]
enum Scheme {
    Http,
    Https,
}

struct ParsedUrl {
    scheme: Scheme,
    host: String,
    port: u16,
    path_and_query: String,
}

/// Parses an `http://` or `https://` URL into its components.
///
/// Returns `Err(reason)` for any URL that fails the security policy checks:
/// non-http(s) scheme, userinfo component, or ambiguous IP representation.
fn parse_url(url: &str) -> Result<ParsedUrl, String> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        (Scheme::Https, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (Scheme::Http, r)
    } else {
        return Err(format!("rejected: non-http(s) scheme in '{url}'"));
    };

    // Split authority from path/query/fragment.
    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], rest[idx..].to_string()),
        None => (rest, "/".to_string()),
    };

    // Reject userinfo (no auth forwarding).
    if authority.contains('@') {
        return Err(format!("rejected: URL contains userinfo '{url}'"));
    }

    // Parse host and port.
    let (host_str, port) = if authority.starts_with('[') {
        // IPv6 literal: [::1] or [::1]:8080
        let end = authority
            .find(']')
            .ok_or_else(|| format!("invalid IPv6 literal in '{url}'"))?;
        let host = authority[1..end].to_string();
        let port = if authority.len() > end + 1 && authority[end + 1..].starts_with(':') {
            authority[end + 2..]
                .parse::<u16>()
                .map_err(|_| format!("invalid port in '{url}'"))?
        } else {
            default_port(scheme)
        };
        (host, port)
    } else if let Some(colon) = authority.rfind(':') {
        if let Ok(port) = authority[colon + 1..].parse::<u16>() {
            (authority[..colon].to_string(), port)
        } else {
            (authority.to_string(), default_port(scheme))
        }
    } else {
        (authority.to_string(), default_port(scheme))
    };

    reject_ambiguous_ip(&host_str)?;

    Ok(ParsedUrl {
        scheme,
        host: host_str,
        port,
        path_and_query,
    })
}

fn default_port(scheme: Scheme) -> u16 {
    match scheme {
        Scheme::Http => 80,
        Scheme::Https => 443,
    }
}

/// Rejects host strings that are ambiguous IP representations.
///
/// Blocked forms:
/// - Hex IPs (`0x7f000001`, `0xC0A80001`)
/// - Octal dotted-decimal (`0177.0.0.1`)
/// - Single-integer IPs (`2130706433`)
fn reject_ambiguous_ip(host: &str) -> Result<(), String> {
    let lower = host.to_ascii_lowercase();

    // Hex: starts with 0x / 0X
    if lower.starts_with("0x") {
        return Err(format!("rejected: hex IP representation '{host}'"));
    }

    // Dotted decimal — check for octal octets (leading zero on a multi-digit part).
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        for part in &parts {
            if part.len() > 1 && part.starts_with('0') {
                return Err(format!("rejected: octal octet in IP '{host}'"));
            }
        }
    }

    // Single-integer representation (e.g. 2130706433 for 127.0.0.1).
    if !host.is_empty() && host.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = host.parse::<u64>() {
            if n <= u32::MAX as u64 {
                return Err(format!("rejected: integer IP representation '{host}'"));
            }
        }
    }

    Ok(())
}

// ── IP blocklist ──────────────────────────────────────────────────────────────

/// Returns `true` when `ip` falls inside a blocked range.
///
/// Blocked ranges: RFC 1918 private, link-local (169.254/16), loopback
/// (127/8, ::1), IPv6 private (fc00::/7), and IPv4-mapped IPv6.
fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10                                        // 10.0.0.0/8
        || (o[0] == 172 && (o[1] & 0xF0) == 16)      // 172.16.0.0/12
        || (o[0] == 192 && o[1] == 168)               // 192.168.0.0/16
        || (o[0] == 169 && o[1] == 254)               // 169.254.0.0/16 link-local
        || o[0] == 127                                // 127.0.0.0/8 loopback
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    let o = ip.octets();
    ip == Ipv6Addr::LOCALHOST                  // ::1
        || (o[0] & 0xFE) == 0xFC              // fc00::/7 private
        || (o[..10] == [0u8; 10] && o[10..12] == [0xff, 0xff]) // ::ffff:0:0/96 IPv4-mapped
}

// ── Core check logic ──────────────────────────────────────────────────────────

/// Checks a single URL, returning a classified result.
///
/// All errors are converted to `Unreachable` or `Rejected` — this function
/// never panics.
fn check_url(url: &str) -> UrlCheckResult {
    match check_url_inner(url) {
        Ok(r) => r,
        Err(msg) => {
            if msg.starts_with("rejected:") {
                UrlCheckResult::Rejected(msg)
            } else {
                UrlCheckResult::Unreachable(msg)
            }
        }
    }
}

fn check_url_inner(url: &str) -> Result<UrlCheckResult, String> {
    let parsed = parse_url(url)?;

    // DNS resolution.
    let addr_str = format!("{}:{}", parsed.host, parsed.port);
    let addrs: Vec<std::net::SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for '{}': {e}", parsed.host))?
        .collect();

    if addrs.is_empty() {
        return Ok(UrlCheckResult::Unreachable(format!(
            "DNS returned no addresses for '{}'",
            parsed.host
        )));
    }

    // Blocklist check — all resolved addresses must pass.
    for addr in &addrs {
        if is_blocked(addr.ip()) {
            return Err(format!(
                "rejected: resolved IP {} is in a blocked range",
                addr.ip()
            ));
        }
    }

    // TCP connect — try each address.
    let tcp = addrs
        .iter()
        .find_map(|addr| TcpStream::connect_timeout(addr, TIMEOUT).ok())
        .ok_or_else(|| {
            format!("connection refused or timed out for '{}'", parsed.host)
        })?;

    tcp.set_read_timeout(Some(TIMEOUT))
        .map_err(|e| format!("set_read_timeout: {e}"))?;
    tcp.set_write_timeout(Some(TIMEOUT))
        .map_err(|e| format!("set_write_timeout: {e}"))?;

    let request = format!(
        "HEAD {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
        parsed.path_and_query, parsed.host
    );

    let status_line = match parsed.scheme {
        Scheme::Http => send_http_head(tcp, &request),
        Scheme::Https => send_https_head(tcp, &parsed.host, &request),
    }?;

    classify_status_line(&status_line)
}

// ── HTTP / HTTPS transports ───────────────────────────────────────────────────

fn send_http_head(mut stream: TcpStream, request: &str) -> Result<String, String> {
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;
    read_status_line(stream)
}

fn send_https_head(tcp: TcpStream, host: &str, request: &str) -> Result<String, String> {
    use rustls::pki_types::ServerName;

    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| format!("invalid TLS server name: '{host}'"))?;

    let root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.iter().cloned().collect(),
    };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let conn = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("TLS setup failed: {e}"))?;

    let mut tls = rustls::StreamOwned::new(conn, tcp);

    tls.write_all(request.as_bytes())
        .map_err(|e| format!("TLS write failed: {e}"))?;

    read_status_line(tls)
}

/// Reads bytes until `\n` or [`STATUS_LINE_MAX`] bytes, stripping `\r`.
fn read_status_line<R: Read>(mut reader: R) -> Result<String, String> {
    let mut buf = Vec::with_capacity(STATUS_LINE_MAX);
    let mut byte = [0u8; 1];

    loop {
        if buf.len() >= STATUS_LINE_MAX {
            break;
        }
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                if byte[0] != b'\r' {
                    buf.push(byte[0]);
                }
            }
            Err(e) => return Err(format!("read failed: {e}")),
        }
    }

    String::from_utf8(buf).map_err(|_| "non-UTF-8 status line".to_string())
}

// ── Status code classification ────────────────────────────────────────────────

fn classify_status_line(line: &str) -> Result<UrlCheckResult, String> {
    // Expected format: "HTTP/1.x NNN Reason Phrase"
    let mut parts = line.splitn(3, ' ');
    let _version = parts.next();
    let code_str = parts
        .next()
        .ok_or_else(|| format!("unexpected response: '{line}'"))?;

    let code: u16 = code_str
        .parse()
        .map_err(|_| format!("invalid status code in response: '{line}'"))?;

    Ok(match code {
        200..=399 | 401 | 403 => UrlCheckResult::Ok,
        404 | 410 => UrlCheckResult::Broken(code),
        500..=599 => UrlCheckResult::PossiblyDown(code),
        _ => UrlCheckResult::Unreachable(format!("unexpected status {code}")),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_url ────────────────────────────────────────────────────────────

    #[test]
    fn parse_url_rejects_non_http_scheme() {
        assert!(parse_url("ftp://example.com").is_err());
        assert!(parse_url("file:///etc/passwd").is_err());
        assert!(parse_url("data:text/plain,hello").is_err());
    }

    #[test]
    fn parse_url_accepts_https_with_default_port() {
        let p = parse_url("https://example.com/path").unwrap();
        assert_eq!(p.host, "example.com");
        assert_eq!(p.port, 443);
        assert_eq!(p.path_and_query, "/path");
    }

    #[test]
    fn parse_url_accepts_http_with_explicit_port() {
        let p = parse_url("http://example.com:8080/api").unwrap();
        assert_eq!(p.port, 8080);
    }

    #[test]
    fn parse_url_defaults_path_to_slash() {
        let p = parse_url("https://example.com").unwrap();
        assert_eq!(p.path_and_query, "/");
    }

    #[test]
    fn parse_url_rejects_userinfo() {
        assert!(parse_url("https://user:pass@example.com").is_err());
    }

    // ── reject_ambiguous_ip ──────────────────────────────────────────────────

    #[test]
    fn reject_hex_ip() {
        assert!(reject_ambiguous_ip("0x7f000001").is_err());
    }

    #[test]
    fn reject_octal_octet() {
        assert!(reject_ambiguous_ip("0177.0.0.1").is_err());
    }

    #[test]
    fn reject_integer_ip() {
        assert!(reject_ambiguous_ip("2130706433").is_err());
    }

    #[test]
    fn allow_normal_dotted_decimal() {
        assert!(reject_ambiguous_ip("93.184.216.34").is_ok());
    }

    #[test]
    fn allow_normal_hostname() {
        assert!(reject_ambiguous_ip("example.com").is_ok());
    }

    // ── is_blocked ───────────────────────────────────────────────────────────

    #[test]
    fn blocks_loopback_v4() {
        assert!(is_blocked("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_rfc1918_10() {
        assert!(is_blocked("10.0.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_rfc1918_172() {
        assert!(is_blocked("172.16.0.1".parse().unwrap()));
        assert!(is_blocked("172.31.255.255".parse().unwrap()));
        assert!(!is_blocked("172.15.0.1".parse().unwrap()));
        assert!(!is_blocked("172.32.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_rfc1918_192_168() {
        assert!(is_blocked("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn blocks_link_local() {
        assert!(is_blocked("169.254.1.1".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv6_loopback() {
        assert!(is_blocked("::1".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv6_private_fc00() {
        assert!(is_blocked("fc00::1".parse().unwrap()));
        assert!(is_blocked("fd12:3456:789a::1".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv4_mapped_ipv6() {
        // ::ffff:127.0.0.1
        let ip: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_blocked(ip));
    }

    #[test]
    fn allows_public_ipv4() {
        assert!(!is_blocked("93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn allows_public_ipv6() {
        assert!(!is_blocked("2001:db8::1".parse().unwrap()));
    }

    // ── classify_status_line ─────────────────────────────────────────────────

    #[test]
    fn classify_200_is_ok() {
        let r = classify_status_line("HTTP/1.1 200 OK").unwrap();
        assert_eq!(r, UrlCheckResult::Ok);
    }

    #[test]
    fn classify_301_is_ok() {
        let r = classify_status_line("HTTP/1.1 301 Moved Permanently").unwrap();
        assert_eq!(r, UrlCheckResult::Ok);
    }

    #[test]
    fn classify_401_is_ok() {
        let r = classify_status_line("HTTP/1.1 401 Unauthorized").unwrap();
        assert_eq!(r, UrlCheckResult::Ok);
    }

    #[test]
    fn classify_403_is_ok() {
        let r = classify_status_line("HTTP/1.1 403 Forbidden").unwrap();
        assert_eq!(r, UrlCheckResult::Ok);
    }

    #[test]
    fn classify_404_is_broken() {
        let r = classify_status_line("HTTP/1.1 404 Not Found").unwrap();
        assert_eq!(r, UrlCheckResult::Broken(404));
    }

    #[test]
    fn classify_410_is_broken() {
        let r = classify_status_line("HTTP/1.1 410 Gone").unwrap();
        assert_eq!(r, UrlCheckResult::Broken(410));
    }

    #[test]
    fn classify_500_is_possibly_down() {
        let r = classify_status_line("HTTP/1.1 500 Internal Server Error").unwrap();
        assert_eq!(r, UrlCheckResult::PossiblyDown(500));
    }

    #[test]
    fn classify_invalid_returns_err() {
        assert!(classify_status_line("not a response").is_err());
    }

    // ── verify_urls ──────────────────────────────────────────────────────────

    #[test]
    fn verify_urls_deduplicates_input() {
        // Provide a URL that will fail (no network in unit tests typically).
        // The key test: two identical URLs → one outcome.
        let urls = vec![
            "http://localhost:19999/nonexistent".to_string(),
            "http://localhost:19999/nonexistent".to_string(),
        ];
        let outcomes = verify_urls(&urls);
        assert_eq!(outcomes.len(), 1);
    }

    #[test]
    fn verify_urls_empty_slice_returns_empty() {
        let outcomes = verify_urls(&[]);
        assert!(outcomes.is_empty());
    }

    #[test]
    fn verify_urls_localhost_is_blocked() {
        let urls = vec!["http://127.0.0.1/".to_string()];
        let outcomes = verify_urls(&urls);
        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(&outcomes[0].result, UrlCheckResult::Rejected(_)),
            "expected Rejected, got {:?}",
            outcomes[0].result
        );
    }
}
