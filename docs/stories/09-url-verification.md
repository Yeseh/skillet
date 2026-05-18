# Story: URL Verification (Opt-in)

## As a
Skill author who links to external documentation and resources

## I want to
Optionally verify that URLs in my skills are reachable

## So that
I catch broken links before they confuse agents or users

## Acceptance Criteria

- [ ] URL verification is off by default
- [ ] Enabled via `verify_urls = true` in `skillet.toml` `[build]` section
- [ ] `--offline` flag disables URL checks regardless of config
- [ ] Verification sends a HEAD request and reads only the status line (max 128 bytes)
- [ ] No response headers or body are read
- [ ] No redirects are followed — 3xx means the URL exists
- [ ] Result classification:
  - DNS failure / connection refused / timeout → `unreachable` (warning)
  - 2xx, 3xx → `ok`
  - 401, 403 → `ok` (exists, auth-gated)
  - 404, 410 → `broken` (warning)
  - 5xx → `possibly-down` (info)
- [ ] URL check failures are warnings (build succeeds), errors with `--strict`

### Security
- [ ] All network code is isolated in `net/url_verify.rs` (single auditable module)
- [ ] Strict URL parser: reject non-http(s), reject ambiguous IP representations (hex, octal, mapped)
- [ ] DNS resolved before connecting; resolved IP checked against blocklist
- [ ] Blocklist: RFC 1918 (10.x, 172.16-31.x, 192.168.x), link-local (169.254.x), loopback (127.x, ::1), IPv6 private (fc00::/7)
- [ ] 5s hard wall-clock timeout per URL
- [ ] Concurrency cap: 5 simultaneous checks
- [ ] TLS cert verification enforced (no option to disable)
- [ ] Minimal request headers, no cookies, no auth
- [ ] Same URL not checked twice per build (result cache)
- [ ] Uses raw `TcpStream` + `rustls` — no HTTP library dependency
