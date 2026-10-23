//! The one HTTP path for geo data (ADR-0014): a blocking rustls client behind the [`Http`]
//! trait, an on-disk cache keyed by the provider's tile address and validated by ETag, and a
//! [`Fetcher`] that runs a batch of requests on as many connections as the link sustains.
//!
//! Adaptation: the fetcher keeps an exponentially weighted throughput estimate and a
//! connection count between `min_conn` and `max_conn`. Every clean response that arrives at a
//! megabyte a second or better earns one more connection; every timeout, connection failure,
//! 429 or 5xx halves the count and backs off (honouring `Retry-After`). Per-request timeouts
//! are sized from the expected bytes and the measured throughput, so a slow link gets patience
//! and a dead one gets a quick retry. Offline mode serves the cache only.
//!
//! ponytail: a scoped thread pool with a mutex-and-condvar gate instead of an async runtime —
//! the domain crates stay free of tokio, and a few dozen tile requests do not need more.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use jd_types::{ErrorCode, JdError, JdResult};

/// What a GET came back with.
#[derive(Debug, Clone)]
pub struct Response {
    /// HTTP status.
    pub status: u16,
    /// `ETag` header, for conditional re-fetches.
    pub etag: Option<String>,
    /// `Retry-After` in seconds when the server sent one.
    pub retry_after_s: Option<u64>,
    /// The body (empty for 304).
    pub body: Vec<u8>,
}

/// Transport failures, split by whether a retry can help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// No response within the deadline.
    Timeout,
    /// DNS, TCP or TLS failure.
    Connect(String),
    /// The body passed the size cap.
    TooLarge(u64),
    /// Anything else (a malformed URL, an I/O error mid-body).
    Other(String),
}

impl HttpError {
    fn retryable(&self) -> bool {
        matches!(self, HttpError::Timeout | HttpError::Connect(_))
    }
}

/// A blocking GET. One implementation talks TLS ([`UreqHttp`]); tests script responses.
pub trait Http: Send + Sync {
    /// GET `url` within `timeout`, refusing bodies past `limit`; `if_none_match` sends an ETag.
    fn get(
        &self,
        url: &str,
        if_none_match: Option<&str>,
        timeout: Duration,
        limit: u64,
    ) -> Result<Response, HttpError>;
}

/// The knobs of the policy; the defaults are the ADR's.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Sent on every request; providers ask for an identifying one.
    pub user_agent: String,
    /// Connections the fetcher never drops below / above.
    pub min_conn: usize,
    /// Connections the fetcher never exceeds.
    pub max_conn: usize,
    /// Attempts per request after the first.
    pub retries: u32,
    /// Largest body accepted, bytes.
    pub size_cap: u64,
    /// Floor of every request's deadline; the ceiling is `max_timeout`.
    pub base_timeout: Duration,
    /// Ceiling of every request's deadline.
    pub max_timeout: Duration,
    /// Serve the cache only; a miss is an error.
    pub offline: bool,
    /// Hosts (exact or parent-domain match) a request may go to. `http://` is accepted for
    /// loopback hosts only, so tests can run a local server.
    pub allow_hosts: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            user_agent: format!(
                "jenga-dunia/{} (+https://github.com/SeedeXR/jenga-dunia)",
                env!("CARGO_PKG_VERSION")
            ),
            min_conn: 1,
            max_conn: 4,
            retries: 4,
            size_cap: 64 << 20,
            base_timeout: Duration::from_secs(8),
            max_timeout: Duration::from_secs(120),
            offline: false,
            allow_hosts: Vec::new(),
        }
    }
}

/// One request of a batch.
#[derive(Debug, Clone)]
pub struct Request {
    /// What to GET.
    pub url: String,
    /// Cache key (`provider/z/x/y`); `None` is never cached.
    pub key: Option<String>,
    /// A size guess for the deadline; 0 = unknown.
    pub expect_bytes: u64,
}

/// A request's outcome.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// The body, from the wire or the cache.
    pub bytes: Vec<u8>,
    /// Served by the disk cache (a fresh body or a 304).
    pub from_cache: bool,
}

/// What a batch cost.
#[derive(
    Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Stats {
    /// Requests in the batch.
    pub requests: usize,
    /// Requests answered from the cache.
    pub cache_hits: usize,
    /// Bytes that crossed the wire (a 304 counts nothing).
    pub bytes: u64,
    /// Retries across the batch.
    pub retries: u32,
    /// Connections in use when the batch ended.
    pub connections: usize,
    /// Throughput estimate at the end, bytes per second.
    pub bytes_per_s: f64,
    /// Wall time of the batch.
    pub elapsed_ms: u64,
}

/// Disk cache: `<dir>/<key>` holds the body, `<dir>/<key>.meta` its ETag. Bounded by
/// `cap_bytes`; the oldest entries go first.
#[derive(Debug, Clone)]
pub struct DiskCache {
    dir: PathBuf,
    cap_bytes: u64,
}

impl DiskCache {
    /// A cache under `dir`, bounded by `cap_bytes`.
    pub fn new(dir: impl Into<PathBuf>, cap_bytes: u64) -> Self {
        DiskCache {
            dir: dir.into(),
            cap_bytes,
        }
    }

    /// Where the cache lives.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, key: &str) -> JdResult<PathBuf> {
        // Keys are provider/z/x/y style: plain components only, no traversal.
        let mut p = self.dir.clone();
        for c in key.split('/') {
            if c.is_empty() || c == "." || c == ".." || c.contains(['\\', ':']) {
                return Err(JdError::invalid(
                    "cache key",
                    format!("`{key}` is not provider/z/x/y"),
                ));
            }
            p.push(c);
        }
        Ok(p)
    }

    /// The cached body and its ETag, if any.
    pub fn get(&self, key: &str) -> JdResult<Option<(Vec<u8>, Option<String>)>> {
        let p = self.path(key)?;
        let Ok(bytes) = std::fs::read(&p) else {
            return Ok(None);
        };
        let etag = std::fs::read_to_string(p.with_extension(meta_ext(&p)))
            .ok()
            .and_then(|m| serde_json::from_str::<Meta>(&m).ok())
            .and_then(|m| m.etag);
        Ok(Some((bytes, etag)))
    }

    /// Store a body with its ETag and evict the oldest past the cap.
    pub fn put(&self, key: &str, bytes: &[u8], etag: Option<&str>) -> JdResult<()> {
        let p = self.path(key)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, bytes)?;
        let meta = Meta {
            etag: etag.map(str::to_owned),
            bytes: bytes.len() as u64,
        };
        std::fs::write(
            p.with_extension(meta_ext(&p)),
            serde_json::to_string(&meta)?,
        )?;
        self.evict()
    }

    /// Bytes on disk.
    pub fn size(&self) -> u64 {
        let mut total = 0;
        visit(&self.dir, &mut |p, m| {
            if !p.extension().is_some_and(|e| e == "meta") {
                total += m.len();
            }
        });
        total
    }

    fn evict(&self) -> JdResult<()> {
        let mut entries: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
        let mut total = 0u64;
        visit(&self.dir, &mut |p, m| {
            if !p.extension().is_some_and(|e| e == "meta") {
                total += m.len();
                entries.push((
                    p.to_path_buf(),
                    m.modified().unwrap_or(std::time::UNIX_EPOCH),
                    m.len(),
                ));
            }
        });
        entries.sort_by_key(|e| e.1);
        for (p, _, len) in entries {
            if total <= self.cap_bytes {
                break;
            }
            let _ = std::fs::remove_file(&p);
            let _ = std::fs::remove_file(p.with_extension(meta_ext(&p)));
            total = total.saturating_sub(len);
        }
        Ok(())
    }
}

fn meta_ext(p: &Path) -> String {
    match p.extension().and_then(|e| e.to_str()) {
        Some(e) => format!("{e}.meta"),
        None => "meta".into(),
    }
}

fn visit(dir: &Path, f: &mut dyn FnMut(&Path, &std::fs::Metadata)) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            visit(&p, f);
        } else if let Ok(m) = e.metadata() {
            f(&p, &m);
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    etag: Option<String>,
    bytes: u64,
}

/// The throughput gauge and the connection count it steers.
#[derive(Debug, Clone)]
struct Gauge {
    conn: usize,
    bytes_per_s: f64,
    last_trouble: Option<Instant>,
}

/// Runs batches under a [`Policy`].
pub struct Fetcher {
    http: Arc<dyn Http>,
    policy: Policy,
    cache: Option<DiskCache>,
    gauge: Mutex<Gauge>,
}

const MIN_ASSUMED_BPS: f64 = 100.0 * 1024.0;
const FAST_BPS: f64 = 1024.0 * 1024.0;

impl Fetcher {
    /// A fetcher over `http` under `policy`, caching in `cache` when given.
    pub fn new(http: Arc<dyn Http>, policy: Policy, cache: Option<DiskCache>) -> Self {
        let conn = policy.min_conn.max(1).min(policy.max_conn.max(1));
        Fetcher {
            http,
            policy,
            cache,
            gauge: Mutex::new(Gauge {
                conn,
                bytes_per_s: 0.0,
                last_trouble: None,
            }),
        }
    }

    /// The policy in force.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    fn gauge(&self) -> std::sync::MutexGuard<'_, Gauge> {
        // A poisoned gauge only means a worker panicked mid-update; the numbers stay usable.
        self.gauge.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check_url(&self, url: &str) -> JdResult<()> {
        let (scheme, rest) = url
            .split_once("://")
            .ok_or_else(|| JdError::invalid("url", format!("`{url}` has no scheme")))?;
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host.rsplit('@').next().unwrap_or(host);
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
        if !(scheme == "https" || (scheme == "http" && loopback)) {
            return Err(JdError::new(
                ErrorCode::E_NETWORK,
                format!("`{url}`: only https:// (http:// for loopback) is fetched"),
            ));
        }
        let allowed = loopback
            || self.policy.allow_hosts.iter().any(|a| {
                let a = a.to_ascii_lowercase();
                host == a || host.ends_with(&format!(".{a}"))
            });
        if !allowed {
            return Err(JdError::new(
                ErrorCode::E_NETWORK,
                format!("host `{host}` is not on the allowlist"),
            )
            .next(
                "Allowed hosts are the ADR-0014 providers; see docs/adr/0014-geo-fetch-policy.md.",
            ));
        }
        Ok(())
    }

    fn timeout_for(&self, expect_bytes: u64) -> Duration {
        let bps = self.gauge().bytes_per_s.max(MIN_ASSUMED_BPS);
        let transfer = Duration::from_secs_f64(2.0 * expect_bytes as f64 / bps);
        (self.policy.base_timeout + transfer).min(self.policy.max_timeout)
    }

    fn note_success(&self, bytes: u64, took: Duration) {
        let mut g = self.gauge();
        if bytes > 0 && took > Duration::ZERO {
            let bps = bytes as f64 / took.as_secs_f64();
            g.bytes_per_s = if g.bytes_per_s == 0.0 {
                bps
            } else {
                0.7 * g.bytes_per_s + 0.3 * bps
            };
        }
        let calm = g
            .last_trouble
            .is_none_or(|t| t.elapsed() > Duration::from_secs(5));
        if calm && g.bytes_per_s >= FAST_BPS && g.conn < self.policy.max_conn {
            g.conn += 1;
        }
    }

    fn note_trouble(&self) {
        let mut g = self.gauge();
        g.conn = (g.conn / 2).max(self.policy.min_conn.max(1));
        g.last_trouble = Some(Instant::now());
    }

    /// One request: cache, conditional GET, retries with backoff.
    pub fn fetch(&self, req: &Request, cancel: &AtomicBool) -> JdResult<(Fetched, u32)> {
        self.check_url(&req.url)?;
        let cached = match (&self.cache, &req.key) {
            (Some(c), Some(k)) => c.get(k)?,
            _ => None,
        };
        if self.policy.offline {
            return match cached {
                Some((bytes, _)) => Ok((
                    Fetched {
                        bytes,
                        from_cache: true,
                    },
                    0,
                )),
                None => Err(JdError::new(
                    ErrorCode::E_NETWORK,
                    format!("offline and `{}` is not cached", req.url),
                )
                .next("Fetch once online, or turn `offline` off.")),
            };
        }
        let etag = cached.as_ref().and_then(|(_, e)| e.clone());
        let mut retries = 0u32;
        let mut attempt = 0u32;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(JdError::new(ErrorCode::E_CANCELLED, "fetch cancelled"));
            }
            let timeout = self.timeout_for(req.expect_bytes);
            let t0 = Instant::now();
            let outcome = self
                .http
                .get(&req.url, etag.as_deref(), timeout, self.policy.size_cap);
            let took = t0.elapsed();
            let (retryable, wait, why) = match outcome {
                Ok(r) if r.status == 304 => {
                    if let Some((bytes, _)) = cached {
                        self.note_success(0, took);
                        return Ok((
                            Fetched {
                                bytes,
                                from_cache: true,
                            },
                            retries,
                        ));
                    }
                    (true, None, "304 without a cached body".to_string())
                }
                Ok(r) if (200..300).contains(&r.status) => {
                    self.note_success(r.body.len() as u64, took);
                    if let (Some(c), Some(k)) = (&self.cache, &req.key) {
                        c.put(k, &r.body, r.etag.as_deref())?;
                    }
                    return Ok((
                        Fetched {
                            bytes: r.body,
                            from_cache: false,
                        },
                        retries,
                    ));
                }
                Ok(r) if r.status == 429 || r.status >= 500 => (
                    true,
                    r.retry_after_s.map(Duration::from_secs),
                    format!("HTTP {}", r.status),
                ),
                Ok(r) => (false, None, format!("HTTP {}", r.status)),
                Err(e) => (e.retryable(), None, format!("{e:?}")),
            };
            if retryable {
                self.note_trouble();
            }
            if !retryable || attempt >= self.policy.retries {
                return Err(JdError::new(
                    ErrorCode::E_NETWORK,
                    format!("{}: {why} after {} attempt(s)", req.url, attempt + 1),
                )
                .next(if retryable {
                    "The server or the link is struggling; try again later."
                } else {
                    "The request itself was refused; check the URL and the provider's terms."
                }));
            }
            let backoff = wait.unwrap_or_else(|| Duration::from_millis(500 << attempt.min(4)));
            std::thread::sleep(backoff.min(Duration::from_secs(30)));
            attempt += 1;
            retries += 1;
        }
    }

    /// A whole batch on the adaptive pool. `progress(done, total)` is called from the calling
    /// thread as results land; the first error cancels the rest.
    pub fn fetch_all(
        &self,
        reqs: &[Request],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> JdResult<(Vec<Fetched>, Stats)> {
        let t0 = Instant::now();
        let total = reqs.len();
        struct Shared {
            next: usize,
            active: usize,
            done: usize,
            results: Vec<Option<Fetched>>,
            error: Option<JdError>,
            retries: u32,
            cache_hits: usize,
            bytes: u64,
        }
        let shared = Mutex::new(Shared {
            next: 0,
            active: 0,
            done: 0,
            results: (0..total).map(|_| None).collect(),
            error: None,
            retries: 0,
            cache_hits: 0,
            bytes: 0,
        });
        let cv = Condvar::new();
        let workers = self.policy.max_conn.max(1).min(total.max(1));
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let idx = {
                            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                            // Wait for a slot under the current connection count.
                            while s.error.is_none()
                                && s.next < total
                                && s.active >= self.gauge().conn
                            {
                                s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
                            }
                            if s.error.is_some() || s.next >= total {
                                break;
                            }
                            let i = s.next;
                            s.next += 1;
                            s.active += 1;
                            i
                        };
                        let r = self.fetch(&reqs[idx], cancel);
                        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                        s.active -= 1;
                        s.done += 1;
                        match r {
                            Ok((f, retries)) => {
                                s.retries += retries;
                                if f.from_cache {
                                    s.cache_hits += 1;
                                } else {
                                    s.bytes += f.bytes.len() as u64;
                                }
                                s.results[idx] = Some(f);
                            }
                            Err(e) => {
                                if s.error.is_none() {
                                    s.error = Some(e);
                                }
                                cancel.store(true, Ordering::Relaxed);
                            }
                        }
                        cv.notify_all();
                    }
                    cv.notify_all();
                });
            }
            // Progress from this thread while the workers run.
            let mut seen = 0;
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            while s.done < total && s.error.is_none() {
                if s.done != seen {
                    seen = s.done;
                    drop(s);
                    progress(seen, total);
                    s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    continue;
                }
                s = cv
                    .wait_timeout(s, Duration::from_millis(100))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        });
        let s = shared.into_inner().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = s.error {
            return Err(e);
        }
        progress(total, total);
        let g = self.gauge();
        let stats = Stats {
            requests: total,
            cache_hits: s.cache_hits,
            bytes: s.bytes,
            retries: s.retries,
            connections: g.conn,
            bytes_per_s: g.bytes_per_s,
            elapsed_ms: t0.elapsed().as_millis() as u64,
        };
        let results = s
            .results
            .into_iter()
            .map(|r| {
                r.unwrap_or(Fetched {
                    bytes: Vec::new(),
                    from_cache: false,
                })
            })
            .collect();
        Ok((results, stats))
    }
}

/// The real client: rustls, gzip, no status-as-error, redirects capped.
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl UreqHttp {
    /// An agent sending `user_agent`.
    pub fn new(user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(user_agent)
            .max_redirects(4)
            .build();
        UreqHttp {
            agent: config.new_agent(),
        }
    }
}

impl Http for UreqHttp {
    fn get(
        &self,
        url: &str,
        if_none_match: Option<&str>,
        timeout: Duration,
        limit: u64,
    ) -> Result<Response, HttpError> {
        let mut rb = self
            .agent
            .get(url)
            .config()
            .timeout_per_call(Some(timeout))
            .build();
        if let Some(e) = if_none_match {
            rb = rb.header("If-None-Match", e);
        }
        let mut resp = rb.call().map_err(|e| match e {
            ureq::Error::Timeout(_) => HttpError::Timeout,
            ureq::Error::HostNotFound
            | ureq::Error::ConnectionFailed
            | ureq::Error::Io(_)
            | ureq::Error::Tls(_) => HttpError::Connect(e.to_string()),
            other => HttpError::Other(other.to_string()),
        })?;
        let status = resp.status().as_u16();
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let etag = header("etag");
        let retry_after_s = header("retry-after").and_then(|v| v.trim().parse::<u64>().ok());
        let body = resp
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(n) => HttpError::TooLarge(n),
                ureq::Error::Timeout(_) => HttpError::Timeout,
                other => HttpError::Other(other.to_string()),
            })?;
        Ok(Response {
            status,
            etag,
            retry_after_s,
            body,
        })
    }
}

/// A scripted client for tests: responses keyed by URL, with optional delays.
#[derive(Default)]
pub struct FakeHttp {
    /// Scripted responses per URL, consumed in order (the last one repeats).
    pub responses: Mutex<HashMap<String, Vec<Result<Response, HttpError>>>>,
    /// Simulated latency per call.
    pub delay: Duration,
    /// Every call made: `(url, If-None-Match)`.
    pub calls: Mutex<Vec<(String, Option<String>)>>,
}

impl Http for FakeHttp {
    fn get(
        &self,
        url: &str,
        if_none_match: Option<&str>,
        _timeout: Duration,
        _limit: u64,
    ) -> Result<Response, HttpError> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((url.to_string(), if_none_match.map(str::to_owned)));
        if self.delay > Duration::ZERO {
            std::thread::sleep(self.delay);
        }
        let mut m = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        let q = m
            .get_mut(url)
            .ok_or_else(|| HttpError::Other(format!("unscripted {url}")))?;
        if q.len() > 1 {
            q.remove(0)
        } else {
            q.first()
                .cloned()
                .unwrap_or(Err(HttpError::Other("empty script".into())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(body: &[u8], etag: Option<&str>) -> Result<Response, HttpError> {
        Ok(Response {
            status: 200,
            etag: etag.map(str::to_owned),
            retry_after_s: None,
            body: body.to_vec(),
        })
    }

    fn fetcher(fake: Arc<FakeHttp>, cache: Option<DiskCache>, offline: bool) -> Fetcher {
        let policy = Policy {
            allow_hosts: vec!["example.org".into()],
            offline,
            retries: 2,
            max_conn: 4,
            ..Policy::default()
        };
        Fetcher::new(fake, policy, cache)
    }

    #[test]
    fn allowlist_and_scheme_are_enforced() {
        let f = fetcher(Arc::new(FakeHttp::default()), None, false);
        let c = AtomicBool::new(false);
        let bad = |u: &str| {
            f.fetch(
                &Request {
                    url: u.into(),
                    key: None,
                    expect_bytes: 0,
                },
                &c,
            )
            .unwrap_err()
            .code
        };
        assert_eq!(bad("https://evil.example.com/x"), ErrorCode::E_NETWORK);
        assert_eq!(bad("http://example.org/x"), ErrorCode::E_NETWORK);
        assert_eq!(bad("ftp://example.org/x"), ErrorCode::E_NETWORK);
        assert!(f.check_url("https://tiles.example.org/1/2/3.png").is_ok());
        assert!(
            f.check_url("http://127.0.0.1:8080/t.png").is_ok(),
            "loopback over http for tests"
        );
    }

    #[test]
    fn cache_etag_304_and_offline() {
        let dir = std::env::temp_dir().join(format!("jd-geo-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let fake = Arc::new(FakeHttp::default());
        fake.responses.lock().unwrap().insert(
            "https://example.org/t/1/2/3.png".into(),
            vec![
                ok(b"tile", Some("\"v1\"")),
                Ok(Response {
                    status: 304,
                    etag: None,
                    retry_after_s: None,
                    body: vec![],
                }),
            ],
        );
        let req = Request {
            url: "https://example.org/t/1/2/3.png".into(),
            key: Some("t/1/2/3.png".into()),
            expect_bytes: 4,
        };
        let c = AtomicBool::new(false);
        let f = fetcher(fake.clone(), Some(DiskCache::new(&dir, 1 << 20)), false);
        let (a, _) = f.fetch(&req, &c).unwrap();
        assert!(!a.from_cache && a.bytes == b"tile");
        let (b, _) = f.fetch(&req, &c).unwrap();
        assert!(
            b.from_cache && b.bytes == b"tile",
            "304 serves the cached body"
        );
        let calls = fake.calls.lock().unwrap();
        assert_eq!(
            calls[1].1.as_deref(),
            Some("\"v1\""),
            "the second GET is conditional"
        );
        drop(calls);
        // Offline: the cache answers; a miss is E_NETWORK.
        let off = fetcher(fake.clone(), Some(DiskCache::new(&dir, 1 << 20)), true);
        assert!(off.fetch(&req, &c).unwrap().0.from_cache);
        let miss = Request {
            url: "https://example.org/t/9/9/9.png".into(),
            key: Some("t/9/9/9.png".into()),
            expect_bytes: 0,
        };
        assert_eq!(off.fetch(&miss, &c).unwrap_err().code, ErrorCode::E_NETWORK);
        // The cap evicts the oldest entry.
        let small = DiskCache::new(&dir, 6);
        small.put("t/a", b"aaaa", None).unwrap();
        small.put("t/b", b"bbbb", None).unwrap();
        assert!(small.get("t/a").unwrap().is_none() && small.get("t/b").unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transient_errors_back_off_and_halve_the_connections_then_recover() {
        let fake = Arc::new(FakeHttp::default());
        let busy = Ok(Response {
            status: 429,
            etag: None,
            retry_after_s: Some(0),
            body: vec![],
        });
        fake.responses.lock().unwrap().insert(
            "https://example.org/a".into(),
            vec![busy.clone(), busy, ok(b"a", None)],
        );
        fake.responses.lock().unwrap().insert(
            "https://example.org/b".into(),
            vec![Err(HttpError::Other("nope".into()))],
        );
        let f = fetcher(fake, None, false);
        {
            f.gauge().conn = 4;
        }
        let c = AtomicBool::new(false);
        let (got, retries) = f
            .fetch(
                &Request {
                    url: "https://example.org/a".into(),
                    key: None,
                    expect_bytes: 1,
                },
                &c,
            )
            .unwrap();
        assert_eq!((got.bytes.as_slice(), retries), (&b"a"[..], 2));
        assert_eq!(f.gauge().conn, 1, "two 429s halve 4 → 2 → 1");
        let e = f
            .fetch(
                &Request {
                    url: "https://example.org/b".into(),
                    key: None,
                    expect_bytes: 1,
                },
                &c,
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::E_NETWORK);
        assert!(
            e.message.contains("1 attempt"),
            "a permanent error is not retried: {}",
            e.message
        );
        // Fast, calm traffic earns connections back.
        f.gauge().last_trouble = None;
        f.note_success(8 << 20, Duration::from_secs(1));
        assert_eq!(f.gauge().conn, 2);
        assert!(
            f.timeout_for(100 << 20) > f.policy.base_timeout,
            "the deadline grows with the expected bytes"
        );
        assert!(f.timeout_for(u64::MAX / 4) <= f.policy.max_timeout);
    }

    #[test]
    fn a_batch_runs_on_the_pool_and_reports_progress_and_stats() {
        let fake = Arc::new(FakeHttp {
            delay: Duration::from_millis(20),
            ..Default::default()
        });
        for i in 0..12 {
            fake.responses.lock().unwrap().insert(
                format!("https://example.org/{i}"),
                vec![ok(&[i as u8; 1000], None)],
            );
        }
        let f = fetcher(fake, None, false);
        let reqs: Vec<Request> = (0..12)
            .map(|i| Request {
                url: format!("https://example.org/{i}"),
                key: None,
                expect_bytes: 1000,
            })
            .collect();
        let c = AtomicBool::new(false);
        let mut ticks = Vec::new();
        let (out, stats) = f
            .fetch_all(&reqs, &c, &mut |d, t| ticks.push((d, t)))
            .unwrap();
        assert_eq!(out.len(), 12);
        assert!(
            out.iter().enumerate().all(|(i, o)| o.bytes[0] == i as u8),
            "results keep request order"
        );
        assert_eq!(stats.requests, 12);
        assert_eq!(stats.bytes, 12_000);
        assert_eq!(ticks.last(), Some(&(12, 12)));
        assert!(ticks.windows(2).all(|w| w[0].0 <= w[1].0));
        // One failure cancels the rest.
        let fake = Arc::new(FakeHttp::default());
        fake.responses
            .lock()
            .unwrap()
            .insert("https://example.org/ok".into(), vec![ok(b"x", None)]);
        fake.responses.lock().unwrap().insert(
            "https://example.org/no".into(),
            vec![Ok(Response {
                status: 404,
                etag: None,
                retry_after_s: None,
                body: vec![],
            })],
        );
        let f = fetcher(fake, None, false);
        let reqs = vec![
            Request {
                url: "https://example.org/no".into(),
                key: None,
                expect_bytes: 0,
            },
            Request {
                url: "https://example.org/ok".into(),
                key: None,
                expect_bytes: 0,
            },
        ];
        let c = AtomicBool::new(false);
        assert_eq!(
            f.fetch_all(&reqs, &c, &mut |_, _| {}).unwrap_err().code,
            ErrorCode::E_NETWORK
        );
    }
}
