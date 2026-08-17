//! Errors, and the mapping from API status codes to user-facing text.
//!
//! SPEC §3.4. The strings here are the contract — they are what the user reads.

use crate::validate::Report;

/// Map an API `base_resp.status_code` to the message shown to the user.
///
/// Unmapped codes fall back to the server's own `status_msg` rather than a
/// generic apology, so an undocumented code is still actionable.
pub fn user_message(code: i64, status_msg: &str) -> String {
    match code {
        1002 => "Rate limited. Free tier allows 3 requests/minute — retry shortly".to_owned(),
        1004 => "Authentication failed — check your API key".to_owned(),
        // SPEC §3.5: observed 2026-08-17 — the `-free` models are subject to
        // the same balance check, so telling the user to switch model sends
        // them down a dead end.
        1008 => "Insufficient balance — add credit to your MiniMax account. \
                 The free-tier models are subject to the same check"
            .to_owned(),
        1026 => "Content flagged as sensitive — revise the prompt or lyrics".to_owned(),
        2013 => "Invalid parameters".to_owned(),
        2049 => "Invalid API key".to_owned(),
        _ if status_msg.is_empty() => format!("MiniMax returned error {code}"),
        _ => format!("MiniMax returned error {code}: {status_msg}"),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// HTTP 200 with `base_resp.status_code != 0`. SPEC §3.3: this is the
    /// common failure shape, not an HTTP-level error.
    #[error("{message}")]
    Api {
        code: i64,
        /// Already mapped for display — see [`user_message`].
        message: String,
        /// The server's own wording, kept for the details expander.
        status_msg: String,
        /// SPEC §3.4: surface this on every error, for support requests.
        trace_id: Option<String>,
    },

    /// The request never completed: connection, TLS, or the 600s timeout.
    #[error("Could not reach MiniMax: {0}")]
    Transport(String),

    /// A 4xx/5xx at the HTTP layer, before any `base_resp` could be read.
    #[error("MiniMax returned HTTP {status}")]
    Http { status: u16, body: String },

    /// The body did not match the documented shape.
    #[error("Could not read the response: {0}")]
    Decode(String),

    /// The user cancelled. SPEC §9: leaves no partial file behind.
    #[error("Cancelled")]
    Cancelled,

    /// Failed client-side validation, so nothing was sent. SPEC §4.
    #[error("{0}")]
    Invalid(Report),
}

impl Error {
    /// The `trace_id`, when the failure got far enough to have one.
    pub fn trace_id(&self) -> Option<&str> {
        match self {
            Error::Api { trace_id, .. } => trace_id.as_deref(),
            _ => None,
        }
    }

    /// The API status code, when this was an API-level failure.
    pub fn code(&self) -> Option<i64> {
        match self {
            Error::Api { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// Whether retrying the identical request could plausibly succeed.
    /// `1002` is the rate limiter; back off and retry (SPEC §4).
    pub fn is_retryable(&self) -> bool {
        match self {
            Error::Api { code, .. } => *code == 1002,
            Error::Transport(_) => true,
            Error::Http { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        // Deliberately formatted without the URL: it carries no key, but the
        // habit of never interpolating request context into logs is the point.
        if e.is_timeout() {
            Error::Transport("the request timed out".to_owned())
        } else {
            Error::Transport(e.without_url().to_string())
        }
    }
}
