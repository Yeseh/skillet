# ADR-0003: URL verification isolated and opt-in

## Status
Accepted

## Context
Skills reference URLs that can break over time. Verifying reachability is valuable but introduces security risks (SSRF, DNS rebinding, slowloris, network posture leakage) and build reproducibility concerns (flaky networks, transient outages).

## Decision
URL verification is:
- **Opt-in** via `verify_urls = true` in `skillet.toml` `[build]` section
- **Isolated** in a dedicated `net/url_verify.rs` module — all network I/O lives here, nowhere else, for easy auditing
- **Hardened** against known attack vectors

### Security controls:
1. Strict URL parser — reject non-http(s), reject ambiguous IP representations (hex, octal, mapped)
2. DNS resolve first, check resolved IP against blocklist before connecting
3. Blocklist: RFC 1918 (10.x, 172.16-31.x, 192.168.x), link-local (169.254.x), loopback (127.x, ::1), IPv6 private (fc00::/7)
4. No redirects followed — a 3xx response means the URL exists; the user can follow the redirect themselves. This also eliminates redirect-based SSRF (attacker redirecting to internal IPs).
5. HEAD request only — read status line only (max 128 bytes), ignore all response headers, close immediately
6. Max 8KB response headers
7. Hard 5s wall-clock timeout per URL
8. Concurrency cap: 5 simultaneous checks
9. TLS cert verification enforced (no disable option)
10. Minimal headers, no cookies, no auth
11. Results cached per build run (same URL not checked twice)
12. `--offline` flag disables all URL checks regardless of config

### Result classification:
- DNS failure / connection refused / timeout → `unreachable` (warning)
- 2xx, 3xx (resolved) → `ok`
- 401, 403 → `ok` (exists, auth-gated)
- 404, 410 → `broken` (warning)
- 5xx → `possibly-down` (info)

### Environment variable access:
Only env vars explicitly declared in `skillet.toml` `[env]` section (with required defaults) are accessed. The full environment is never queried.

## Consequences
- Safe by default (off until explicitly enabled)
- Auditable (single module, no network code elsewhere)
- Builds remain reproducible (opt-in, warnings not errors, cached)
- SSRF mitigated even when enabled (IP blocklist, redirect re-validation)
