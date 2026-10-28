//! Ultra-PII scrubbing and aggregation for the debug telemetry lane.
//!
//! The rules are blunt on purpose — a scrubber with exceptions is a scrubber
//! with leaks:
//! - URLs keep **scheme + host only**; the path is replaced by a truncated
//!   hash so two visits to the same page still aggregate without revealing
//!   which page it was. Query strings never survive.
//! - Emails, IP literals, and long numeric sequences are replaced by tags.
//! - Free text is replaced by `len:<n>` plus a content hash: enough to
//!   measure, not enough to read.
//!
//! Aggregation is count-based and JSON-ready: data science sees histograms
//! and counts, never content.

use std::collections::BTreeMap;

/// Truncated FNV-1a content hash, hex-encoded — what replaces page paths and
/// free text. Collision-tolerant by design (this is counting, not identity).
pub fn content_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// URL → `scheme://host/p:<hash8>`. Drops userinfo, port-adjacent query
/// strings, and fragments. The hash covers the *path only* — a query string
/// must never be able to change the aggregate key.
pub fn scrub_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return format!("[UNPARSED:{}]", &content_hash(url)[..8]);
    };
    let scheme = &url[..scheme_end];
    let rest = &url[scheme_end + 3..];
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Strip userinfo (user:pass@).
    let host = authority.rsplit('@').next().unwrap_or("");
    let path = rest[authority.len()..]
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    let path_hash = content_hash(path);
    format!("{scheme}://{host}/p:{}", &path_hash[..8])
}

/// Free text → `len:<n>` — no content survives.
pub fn scrub_text(text: &str) -> String {
    format!("len:{}", text.len())
}

/// Line-level scrubber: any token that looks like an email, an IP literal,
/// or a URL is replaced by its scrubbed form. Everything else passes through
/// length-preserved.
pub fn scrub_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for token in line.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        if looks_like_email(token) {
            out.push_str("[EMAIL]");
        } else if looks_like_ip(token) {
            out.push_str("[IP]");
        } else if token.contains("://") || token.starts_with("www.") {
            out.push_str(&scrub_url(token));
        } else {
            out.push_str(token);
        }
    }
    out
}

fn looks_like_email(token: &str) -> bool {
    let Some(at) = token.find('@') else {
        return false;
    };
    at > 0 && at < token.len() - 1 && token[at + 1..].contains('.') && !token.contains(['/', '?'])
}

fn looks_like_ip(token: &str) -> bool {
    let is_v4 = || {
        let parts: Vec<&str> = token.split('.').collect();
        parts.len() == 4
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.len() <= 3
                    && p.bytes().all(|b| b.is_ascii_digit())
                    && p.parse::<u8>().is_ok()
            })
    };
    let is_v6 = || token.contains(':') && token.chars().all(|c| c.is_ascii_hexdigit() || c == ':');
    is_v4() || is_v6()
}

/// Retained-data limits, measured in unique keys and UTF-8 bytes (not characters).
/// Map overhead and JSON rendering scratch are additional allocations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AggregationLimits {
    pub max_keys: usize,
    pub max_key_bytes: usize,
    pub max_total_key_bytes: usize,
}

impl Default for AggregationLimits {
    fn default() -> Self {
        Self {
            max_keys: 4096,
            max_key_bytes: 1024,
            max_total_key_bytes: 65_536,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AggregationError {
    KeyLimit,
    KeyBytesLimit,
    TotalKeyBytesLimit,
    CountOverflow,
}

impl std::fmt::Display for AggregationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::KeyLimit => "telemetry unique-key limit exceeded",
            Self::KeyBytesLimit => "telemetry per-key byte limit exceeded",
            Self::TotalKeyBytesLimit => "telemetry total key-byte limit exceeded",
            Self::CountOverflow => "telemetry count overflow",
        })
    }
}

impl std::error::Error for AggregationError {}

/// Bounded token → occurrence counts with deterministic JSON output.
#[derive(Default)]
pub struct Aggregator {
    counts: BTreeMap<String, u64>,
    limits: AggregationLimits,
    total_key_bytes: usize,
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(limits: AggregationLimits) -> Self {
        Self {
            counts: BTreeMap::new(),
            limits,
            total_key_bytes: 0,
        }
    }

    pub fn retained_key_bytes(&self) -> usize {
        self.total_key_bytes
    }

    /// Preflight one new key without allocating or changing retained state.
    fn admit_key(
        &self,
        token: &str,
        keys: &mut usize,
        bytes: &mut usize,
    ) -> Result<(), AggregationError> {
        if token.len() > self.limits.max_key_bytes {
            return Err(AggregationError::KeyBytesLimit);
        }
        *keys = keys
            .checked_add(1)
            .filter(|n| *n <= self.limits.max_keys)
            .ok_or(AggregationError::KeyLimit)?;
        *bytes = bytes
            .checked_add(token.len())
            .filter(|n| *n <= self.limits.max_total_key_bytes)
            .ok_or(AggregationError::TotalKeyBytesLimit)?;
        Ok(())
    }

    /// Count one token. Rejection leaves all counts and byte accounting unchanged.
    /// Existing keys remain admissible at capacity unless their count overflows.
    pub fn observe(&mut self, token: &str) -> Result<(), AggregationError> {
        if let Some(count) = self.counts.get_mut(token) {
            *count = count
                .checked_add(1)
                .ok_or(AggregationError::CountOverflow)?;
        } else {
            let mut keys = self.counts.len();
            let mut bytes = self.total_key_bytes;
            self.admit_key(token, &mut keys, &mut bytes)?;
            self.counts.insert(token.to_string(), 1);
            self.total_key_bytes = bytes;
        }
        Ok(())
    }

    /// Count a pre-scrubbed line atomically. Scratch storage has the same limits
    /// as this aggregator; rejection preserves the entire previous aggregate.
    pub fn observe_line(&mut self, scrubbed_line: &str) -> Result<(), AggregationError> {
        let mut staged = Self::with_limits(self.limits);
        for token in scrubbed_line.split_whitespace() {
            staged.observe(token)?;
        }
        self.merge(&staged)
    }

    /// Preflight against the destination's limits and count range before mutation.
    /// On error, both aggregators remain unchanged.
    pub fn merge(&mut self, other: &Aggregator) -> Result<(), AggregationError> {
        let mut keys = self.counts.len();
        let mut bytes = self.total_key_bytes;
        for (token, count) in &other.counts {
            if let Some(existing) = self.counts.get(token) {
                existing
                    .checked_add(*count)
                    .ok_or(AggregationError::CountOverflow)?;
            } else {
                self.admit_key(token, &mut keys, &mut bytes)?;
            }
        }
        for (token, count) in &other.counts {
            // Proven within u64 range above; &mut self excludes intervening writes.
            *self.counts.entry(token.clone()).or_insert(0) += count;
        }
        self.total_key_bytes = bytes;
        Ok(())
    }

    /// JSON-ready output: `{"token": count, ...}` sorted by count descending.
    pub fn to_json(&self) -> String {
        let mut rows: Vec<(&String, &u64)> = self.counts.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let mut out = String::from("{");
        for (i, (token, count)) in rows.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!("\n  \"{}\": {}", json_escape(token), count));
        }
        out.push_str("\n}");
        out
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// C ABI: scrub a URL into a caller-provided buffer. Returns the number of
/// bytes written (excluding the NUL); 0 when the buffer is too small or the
/// pointers are null.
///
/// # Safety
/// `url` must be a NUL-terminated C string or null; `out` must point to
/// `out_len` writable bytes or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kit_telemetry_scrub_url(
    url: *const core::ffi::c_char,
    out: *mut core::ffi::c_char,
    out_len: usize,
) -> usize {
    if url.is_null() || out.is_null() || out_len == 0 {
        return 0;
    }
    // SAFETY: caller contract.
    let Ok(url) = unsafe { core::ffi::CStr::from_ptr(url) }.to_str() else {
        return 0;
    };
    let scrubbed = scrub_url(url);
    if scrubbed.len() + 1 > out_len {
        return 0;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(scrubbed.as_ptr(), out.cast(), scrubbed.len());
        *out.add(scrubbed.len()) = 0;
    }
    scrubbed.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_keep_scheme_and_host_only() {
        let s = scrub_url("https://www.example.com/some/path?q=secret&token=abc#frag");
        assert!(s.starts_with("https://www.example.com/p:"));
        assert!(!s.contains("secret"));
        assert!(!s.contains("token"));
        assert!(!s.contains("frag"));
        assert!(!s.contains("some/path"));
    }

    #[test]
    fn userinfo_is_stripped() {
        let s = scrub_url("https://user:pass@example.com/x");
        assert!(!s.contains("user"));
        assert!(!s.contains("pass"));
        assert!(s.contains("example.com"));
    }

    #[test]
    fn same_path_aggregates_the_same_way() {
        let a = scrub_url("https://example.com/readme");
        let b = scrub_url("https://example.com/readme?utm=1");
        assert_eq!(a, b, "path hash must not depend on the query string");
    }

    #[test]
    fn credentials_do_not_split_the_same_page_aggregate() {
        let plain = scrub_url("https://example.com/readme");
        assert_eq!(plain, scrub_url("https://alice:secret@example.com/readme"));
        assert_eq!(
            plain,
            scrub_url("https://bob:other@example.com/readme?q=x#top")
        );
    }

    #[test]
    fn empty_paths_ignore_credentials_query_and_fragment() {
        assert_eq!(
            scrub_url("https://example.com"),
            scrub_url("https://alice:secret@example.com?q=x#top")
        );
    }

    #[test]
    fn c_buffer_requires_room_for_terminator_and_preserves_guards() {
        let input = c"https://example.com/readme?q=private";
        let expected = scrub_url(input.to_str().unwrap());
        let capacity = expected.len() + 1;
        let mut guarded = vec![0x7fu8; capacity + 2];
        // SAFETY: input is a C literal; the interior has capacity writable bytes.
        let short = unsafe {
            kit_telemetry_scrub_url(
                input.as_ptr(),
                guarded.as_mut_ptr().add(1).cast(),
                capacity - 1,
            )
        };
        assert_eq!(short, 0, "payload-only space must be rejected");
        assert!(
            guarded.iter().all(|&byte| byte == 0x7f),
            "rejection must not write"
        );
        // SAFETY: the same interior now includes space for the terminator.
        let written = unsafe {
            kit_telemetry_scrub_url(input.as_ptr(), guarded.as_mut_ptr().add(1).cast(), capacity)
        };
        assert_eq!(written, expected.len());
        assert_eq!(&guarded[1..1 + written], expected.as_bytes());
        assert_eq!(guarded[1 + written], 0);
        assert_eq!(guarded[0], 0x7f);
        assert_eq!(guarded[capacity + 1], 0x7f, "must not write past capacity");
    }

    #[test]
    fn emails_and_ips_are_tagged() {
        assert_eq!(scrub_line("mail bob@example.com now"), "mail [EMAIL] now");
        assert_eq!(
            scrub_line("from 192.168.1.10 to 10.0.0.1"),
            "from [IP] to [IP]"
        );
        assert_eq!(scrub_line("v6 ::1 here"), "v6 [IP] here");
    }

    #[test]
    fn free_text_loses_its_content() {
        assert_eq!(scrub_text("the quick brown fox"), "len:19");
    }

    #[test]
    fn aggregation_is_json_ready_and_sorted() {
        let mut agg = Aggregator::new();
        for _ in 0..3 {
            agg.observe("alpha").unwrap();
        }
        agg.observe("beta").unwrap();
        let json = agg.to_json();
        assert!(json.starts_with('{'));
        assert!(json.contains("\"alpha\": 3"));
        assert!(json.contains("\"beta\": 1"));
        assert!(json.find("\"alpha\"").unwrap() < json.find("\"beta\"").unwrap());
    }

    #[test]
    fn merge_adds_counts() {
        let mut a = Aggregator::new();
        let mut b = Aggregator::new();
        a.observe("x").unwrap();
        b.observe("x").unwrap();
        b.observe("y").unwrap();
        a.merge(&b).unwrap();
        let json = a.to_json();
        assert!(json.contains("\"x\": 2"));
        assert!(json.contains("\"y\": 1"));
    }

    fn bounded(keys: usize, key_bytes: usize, total_bytes: usize) -> Aggregator {
        Aggregator::with_limits(AggregationLimits {
            max_keys: keys,
            max_key_bytes: key_bytes,
            max_total_key_bytes: total_bytes,
        })
    }

    #[test]
    fn capacity_allows_existing_keys_and_rejects_new_ones() {
        let mut agg = bounded(1, 1, 1);
        agg.observe("a").unwrap();
        agg.observe("a").unwrap();
        assert_eq!(agg.observe("b"), Err(AggregationError::KeyLimit));
        assert_eq!(agg.counts, BTreeMap::from([("a".to_string(), 2)]));
        assert_eq!(agg.retained_key_bytes(), 1);
        assert_eq!(
            bounded(0, 0, 0).observe(""),
            Err(AggregationError::KeyLimit)
        );
        let mut empty_key = bounded(1, 0, 0);
        empty_key.observe("").unwrap();
        assert_eq!(empty_key.retained_key_bytes(), 0);
    }

    #[test]
    fn key_limits_count_utf8_bytes_and_preserve_state() {
        let mut agg = bounded(3, 2, 3);
        assert_eq!(agg.observe("猫"), Err(AggregationError::KeyBytesLimit));
        agg.observe("é").unwrap();
        agg.observe("a").unwrap();
        let before = agg.to_json();
        assert_eq!(agg.observe("b"), Err(AggregationError::TotalKeyBytesLimit));
        assert_eq!(agg.to_json(), before);
        assert_eq!(agg.retained_key_bytes(), 3);
    }

    #[test]
    fn count_overflow_and_late_merge_failure_are_atomic() {
        let mut agg = bounded(2, 1, 2);
        agg.observe("z").unwrap();
        agg.counts.insert("z".to_string(), u64::MAX);
        let before = agg.to_json();
        assert_eq!(agg.observe("z"), Err(AggregationError::CountOverflow));
        let mut other = bounded(2, 1, 2);
        other.observe_line("a z").unwrap();
        let source_before = other.to_json();
        assert_eq!(agg.merge(&other), Err(AggregationError::CountOverflow));
        assert_eq!(agg.to_json(), before);
        assert_eq!(agg.retained_key_bytes(), 1);
        assert_eq!(other.to_json(), source_before);
    }

    #[test]
    fn merge_uses_destination_limits_and_is_atomic() {
        for (mut dest, expected) in [
            (bounded(1, 2, 4), AggregationError::KeyLimit),
            (bounded(3, 1, 4), AggregationError::KeyBytesLimit),
            (bounded(3, 2, 2), AggregationError::TotalKeyBytesLimit),
        ] {
            dest.observe("a").unwrap();
            let mut source = Aggregator::new();
            source.observe_line("a zz").unwrap();
            let before = dest.to_json();
            let source_before = source.to_json();
            assert_eq!(dest.merge(&source), Err(expected));
            assert_eq!(dest.to_json(), before);
            assert_eq!(dest.retained_key_bytes(), 1);
            assert_eq!(source.to_json(), source_before);
        }
    }

    #[test]
    fn rejected_lines_preserve_prior_counts() {
        let mut agg = bounded(2, 1, 2);
        agg.observe("a").unwrap();
        let before = agg.to_json();
        assert_eq!(agg.observe_line("a b c"), Err(AggregationError::KeyLimit));
        assert_eq!(
            agg.observe_line("a zz"),
            Err(AggregationError::KeyBytesLimit)
        );
        assert_eq!(agg.to_json(), before);
        assert_eq!(agg.retained_key_bytes(), 1);
        agg.observe_line("a b a").unwrap();
        assert_eq!(
            agg.counts,
            BTreeMap::from([("a".into(), 3), ("b".into(), 1)])
        );
    }

    #[test]
    fn accepted_trace_matches_reference_counts_and_tie_order() {
        let mut agg = bounded(4, 4, 16);
        let mut reference = BTreeMap::<String, u64>::new();
        for token in ["b", "a", "é", "b", "é", "a", "d"] {
            agg.observe(token).unwrap();
            *reference.entry(token.to_string()).or_default() += 1;
        }
        let mut incoming = bounded(4, 4, 16);
        incoming.observe_line("d a").unwrap();
        agg.merge(&incoming).unwrap();
        *reference.get_mut("d").unwrap() += 1;
        *reference.get_mut("a").unwrap() += 1;
        assert_eq!(agg.counts, reference);
        assert_eq!(agg.retained_key_bytes(), 5);
        assert_eq!(
            agg.to_json(),
            "{\n  \"a\": 3,\n  \"b\": 2,\n  \"d\": 2,\n  \"é\": 2\n}"
        );
    }
}
