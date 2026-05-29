// Secret redaction for content captured from the target repo before it reaches
// the session log. Three layers applied in specificity order: known-prefix token
// shapes, tagged key/value assignments, and an opt-in high-entropy heuristic.
// The marker is `[REDACTED:<type>]`; redaction is irreversible (no length hints).
//
// This is net-new for rexyMCP. Rexy's running masker (ai/filter.rs) is a
// daemon-oriented design (global pattern registry, atomic counters, tracing);
// rexyMCP wants an instance-held redactor with none of that. The regexes here
// borrow Rexy's battle-tested shapes; the structure does not.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{Captures, Regex};

/// A class of secret, naming the `[REDACTED:<tag>]` marker it is masked with.
/// The tag spelling is load-bearing — M5 log-query tools filter on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    OpenAiKey,
    GitHubToken,
    AwsAccessKey,
    SlackToken,
    GoogleToken,
    StripeKey,
    Jwt,
    PrivateKey,
    DbUrl,
    TaggedValue,
    HighEntropy,
}

impl SecretKind {
    /// The full replacement marker, e.g. `[REDACTED:openai_key]`.
    fn marker(self) -> &'static str {
        match self {
            SecretKind::OpenAiKey => "[REDACTED:openai_key]",
            SecretKind::GitHubToken => "[REDACTED:github_token]",
            SecretKind::AwsAccessKey => "[REDACTED:aws_access_key]",
            SecretKind::SlackToken => "[REDACTED:slack_token]",
            SecretKind::GoogleToken => "[REDACTED:google_token]",
            SecretKind::StripeKey => "[REDACTED:stripe_key]",
            SecretKind::Jwt => "[REDACTED:jwt]",
            SecretKind::PrivateKey => "[REDACTED:private_key]",
            SecretKind::DbUrl => "[REDACTED:db_url]",
            SecretKind::TaggedValue => "[REDACTED:tagged_value]",
            SecretKind::HighEntropy => "[REDACTED:high_entropy]",
        }
    }
}

static PRIVATE_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z ]+PRIVATE KEY-----[\s\S]*?-----END [A-Z ]+PRIVATE KEY-----")
        .unwrap()
});

static JWT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap());

static OPENAI_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sk-(?:ant-)?[A-Za-z0-9_-]{20,}").unwrap());

static GITHUB_CLASSIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"gh[psohr]_[A-Za-z0-9]{36}").unwrap());

static GITHUB_PAT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"github_pat_[A-Za-z0-9_]{22,}").unwrap());

static AWS_KEY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").unwrap());

static SLACK_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"xox[baprs]-[A-Za-z0-9-]{10,}").unwrap());

static GOOGLE_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ya29\.[A-Za-z0-9_-]+").unwrap());

static STRIPE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[sr]k_live_[A-Za-z0-9]{16,}").unwrap());

static DB_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(postgresql|postgres|mysql|mongodb(\+srv)?|redis|amqps?|rabbitmq)://[^:@\s]+:[^@\s]+@\S+",
    )
    .unwrap()
});

static TAGGED_KV_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(password|passwd|secret|token|api[_-]?key|apikey)\s*[=:]\s*\S+").unwrap()
});

static URL_PARAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[?&](password|passwd|secret|token|api[_-]?key|apikey)=[^\s&]+").unwrap()
});

static BEARER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)bearer\s+\S+").unwrap());

static ENTROPY_CANDIDATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/=_]{24,}").unwrap());

const ENTROPY_THRESHOLD_BITS: f64 = 4.0;

/// Ordered (pattern, kind) pairs. Specific prefix shapes precede the generic
/// tagged-value patterns so a precisely-matched token gets its specific kind.
fn layered_patterns() -> [(&'static Regex, SecretKind); 13] {
    [
        (&PRIVATE_KEY_RE, SecretKind::PrivateKey),
        (&JWT_RE, SecretKind::Jwt),
        (&OPENAI_KEY_RE, SecretKind::OpenAiKey),
        (&GITHUB_CLASSIC_RE, SecretKind::GitHubToken),
        (&GITHUB_PAT_RE, SecretKind::GitHubToken),
        (&AWS_KEY_RE, SecretKind::AwsAccessKey),
        (&SLACK_TOKEN_RE, SecretKind::SlackToken),
        (&GOOGLE_TOKEN_RE, SecretKind::GoogleToken),
        (&STRIPE_KEY_RE, SecretKind::StripeKey),
        (&DB_URL_RE, SecretKind::DbUrl),
        (&TAGGED_KV_RE, SecretKind::TaggedValue),
        (&URL_PARAM_RE, SecretKind::TaggedValue),
        (&BEARER_RE, SecretKind::TaggedValue),
    ]
}

/// Masks secrets in text before it is written to the session log.
pub struct Redactor {
    high_entropy: bool,
}

impl Redactor {
    /// Built-in prefix + tagged-value patterns; the high-entropy layer is OFF.
    pub fn new() -> Self {
        Self {
            high_entropy: false,
        }
    }

    /// Enable the opt-in high-entropy layer (off by default — it false-positives
    /// on UUIDs, hashes, and build IDs).
    pub fn with_high_entropy(mut self) -> Self {
        self.high_entropy = true;
        self
    }

    /// Replace every detected secret with its `[REDACTED:<type>]` marker.
    pub fn redact(&self, text: &str) -> String {
        let mut out: Cow<str> = Cow::Borrowed(text);
        for (re, kind) in layered_patterns() {
            if re.is_match(out.as_ref()) {
                let replaced = re.replace_all(out.as_ref(), kind.marker()).into_owned();
                out = Cow::Owned(replaced);
            }
        }
        if self.high_entropy {
            out = Cow::Owned(redact_high_entropy(out.as_ref()));
        }
        out.into_owned()
    }
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

fn redact_high_entropy(text: &str) -> String {
    ENTROPY_CANDIDATE_RE
        .replace_all(text, |caps: &Captures<'_>| {
            let candidate = &caps[0];
            if shannon_entropy(candidate) >= ENTROPY_THRESHOLD_BITS {
                SecretKind::HighEntropy.marker().to_string()
            } else {
                candidate.to_string()
            }
        })
        .into_owned()
}

/// Shannon entropy in bits per character over the byte distribution of `s`.
fn shannon_entropy(s: &str) -> f64 {
    let mut counts = [0u32; 256];
    for b in s.bytes() {
        counts[b as usize] += 1;
    }
    let len = s.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / len;
            -p * p.log2()
        })
        .sum::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENAI: &str = "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
    const OPENAI_ANT: &str = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
    const GITHUB_CLASSIC: &str = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefgh01";
    const GITHUB_PAT: &str = "github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz";
    const AWS: &str = "AKIAIOSFODNN7EXAMPLE";
    const SLACK: &str = "xoxb-123456789012-abcdefABCDEF";
    const GOOGLE: &str = "ya29.a0AfH6SMBx-fakegoogletoken-value0123";
    const STRIPE: &str = "sk_live_4eC39HqLyjWDarjtT1zdp7dc";
    const JWT: &str = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    const PEM: &str =
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEAabc123\n-----END RSA PRIVATE KEY-----";
    const DB_URL: &str = "postgres://admin:s3cr3tpw@db.internal:5432/prod";
    const UUID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const GIT_SHA: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b";
    const HIGH_ENTROPY_TOKEN: &str = "Xa9Kd2Lp0QzVbN7mEwR4tYuIoP3sFgHjKlZxCvBnM1q";

    fn assert_redacted(input: &str, expected_marker: &str) {
        let out = Redactor::new().redact(input);
        assert!(
            out.contains(expected_marker),
            "expected {expected_marker} in output, got: {out}"
        );
    }

    #[test]
    fn redacts_openai_key() {
        assert_redacted(OPENAI, "[REDACTED:openai_key]");
        assert_redacted(OPENAI_ANT, "[REDACTED:openai_key]");
    }

    #[test]
    fn redacts_github_token() {
        assert_redacted(GITHUB_CLASSIC, "[REDACTED:github_token]");
        assert_redacted(GITHUB_PAT, "[REDACTED:github_token]");
    }

    #[test]
    fn redacts_aws_access_key() {
        assert_redacted(AWS, "[REDACTED:aws_access_key]");
    }

    #[test]
    fn redacts_slack_token() {
        assert_redacted(SLACK, "[REDACTED:slack_token]");
    }

    #[test]
    fn redacts_google_token() {
        assert_redacted(GOOGLE, "[REDACTED:google_token]");
    }

    #[test]
    fn redacts_stripe_key() {
        assert_redacted(STRIPE, "[REDACTED:stripe_key]");
    }

    #[test]
    fn redacts_jwt() {
        assert_redacted(JWT, "[REDACTED:jwt]");
    }

    #[test]
    fn redacts_private_key_pem_block() {
        assert_redacted(PEM, "[REDACTED:private_key]");
    }

    #[test]
    fn redacts_db_url_with_embedded_credentials() {
        assert_redacted(DB_URL, "[REDACTED:db_url]");
    }

    #[test]
    fn redacts_tagged_value() {
        assert_redacted("api_key = hunter2dummyvalue", "[REDACTED:tagged_value]");
        assert_redacted("password: s3cretdummyvalue", "[REDACTED:tagged_value]");
        assert_redacted(
            "Authorization: bearer abc.def.ghijklmnop",
            "[REDACTED:tagged_value]",
        );
        assert_redacted(
            "https://x.test/cb?token=abc123def456",
            "[REDACTED:tagged_value]",
        );
    }

    #[test]
    fn redacted_output_never_contains_the_raw_secret() {
        let secrets = [
            OPENAI,
            OPENAI_ANT,
            GITHUB_CLASSIC,
            GITHUB_PAT,
            AWS,
            SLACK,
            GOOGLE,
            STRIPE,
            JWT,
            PEM,
            DB_URL,
        ];
        let redactor = Redactor::new();
        for secret in secrets {
            let out = redactor.redact(secret);
            assert!(!out.contains(secret), "full secret leaked in output: {out}");
            let bytes = secret.as_bytes();
            for window in bytes.windows(8) {
                if let Ok(slice) = std::str::from_utf8(window) {
                    assert!(
                        !out.contains(slice),
                        "8-char slice {slice:?} of secret leaked in output: {out}"
                    );
                }
            }
        }
    }

    #[test]
    fn leaves_plain_prose_untouched() {
        let prose = "the password rotation policy is documented in the wiki";
        assert_eq!(Redactor::new().redact(prose), prose);
    }

    #[test]
    fn does_not_redact_uuid_or_git_sha_without_entropy_layer() {
        let redactor = Redactor::new();
        assert_eq!(redactor.redact(UUID), UUID);
        assert_eq!(redactor.redact(GIT_SHA), GIT_SHA);
    }

    #[test]
    fn does_not_redact_normal_identifier() {
        // Words that merely start like a prefix but lack the real token grammar
        // (no hyphen after `sk`, no dot after `ya29`) must pass through.
        let text = "the sktech library and the ya29things module are unrelated";
        assert_eq!(Redactor::new().redact(text), text);
    }

    #[test]
    fn high_entropy_layer_masks_only_when_enabled() {
        let off = Redactor::new();
        let on = Redactor::new().with_high_entropy();

        assert_eq!(
            off.redact(HIGH_ENTROPY_TOKEN),
            HIGH_ENTROPY_TOKEN,
            "entropy layer must be off under new()"
        );
        assert!(
            on.redact(HIGH_ENTROPY_TOKEN)
                .contains("[REDACTED:high_entropy]"),
            "entropy layer must mask a high-entropy token when enabled"
        );
        assert_eq!(
            on.redact(UUID),
            UUID,
            "dashed UUID stays below the entropy candidate length even when enabled"
        );
    }
}
