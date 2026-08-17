//! HTTP transport. SPEC §2 (all HTTP in Rust), §3.3 (no polling, cancellable).

use crate::error::Error;
use crate::request::{CoverPreprocessRequest, GenerationRequest};
use crate::response::{
    parse_cover_preprocess, parse_generation, Audio, CoverPreprocess, Generation,
};
use crate::validate;
use std::fmt;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const DEFAULT_BASE_URL: &str = "https://api.minimax.io";

/// SPEC §3.3: generation is one long synchronous request, so the timeout has
/// to cover the whole job rather than a typical response.
pub const GENERATE_TIMEOUT: Duration = Duration::from_secs(600);
pub const PREPROCESS_TIMEOUT: Duration = Duration::from_secs(120);
/// Downloading a finished file is ordinary I/O, not a generation wait.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

pub struct Client {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

/// SPEC §9: the key must not appear in any log line. A derived `Debug` on a
/// struct holding it is the easiest way to break that, so it is written by hand.
impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl Client {
    pub fn new(api_key: impl Into<String>) -> Result<Self, Error> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("musicmaxxer/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Client {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key: api_key.into(),
        })
    }

    /// `POST /v1/music_generation`.
    ///
    /// Validates first (SPEC §4), then runs one long request. Cancelling drops
    /// the request in flight; nothing is written, so there is no partial file
    /// to clean up (SPEC §11).
    pub async fn generate(
        &self,
        req: &GenerationRequest,
        cancel: &CancellationToken,
    ) -> Result<Generation, Error> {
        let report = validate::generation(req);
        if !report.is_ok() {
            return Err(Error::Invalid(report));
        }

        let body = self
            .send("/v1/music_generation", req, GENERATE_TIMEOUT, cancel)
            .await?;

        parse_generation(&body, req.output_format)
    }

    /// `POST /v1/music_cover_preprocess`. SPEC §3.2 — free, and deduplicated
    /// server-side by content MD5, so cache by content hash before calling.
    pub async fn cover_preprocess(
        &self,
        req: &CoverPreprocessRequest,
        cancel: &CancellationToken,
    ) -> Result<CoverPreprocess, Error> {
        let report = validate::preprocess_model(req.model);
        if !report.is_ok() {
            return Err(Error::Invalid(report));
        }

        let body = self
            .send(
                "/v1/music_cover_preprocess",
                req,
                PREPROCESS_TIMEOUT,
                cancel,
            )
            .await?;

        parse_cover_preprocess(&body)
    }

    /// Turn whatever came back into bytes on disk.
    ///
    /// SPEC §6.4: `output_format` is an internal decision. We request `url` and
    /// download immediately, because the link expires after 24 hours and the
    /// user must never be left holding one. A `hex` response is already bytes
    /// and passes straight through.
    pub async fn fetch_audio(
        &self,
        audio: &Audio,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, Error> {
        let url = match audio {
            Audio::Bytes(b) => return Ok(b.clone()),
            Audio::Url(u) => u,
        };

        // No auth header: this is a pre-signed CDN link, and sending the key to
        // a host we did not choose would leak it.
        let request = self.http.get(url).timeout(DOWNLOAD_TIMEOUT);

        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            res = request.send() => res?,
        };

        let status = response.status();
        if !status.is_success() {
            return Err(Error::Http {
                status: status.as_u16(),
                // The link may simply have expired; say so rather than dumping
                // a CDN error page at the user.
                body: if status.as_u16() == 403 || status.as_u16() == 404 {
                    "the download link has expired (they last 24 hours)".to_owned()
                } else {
                    "could not download the finished audio".to_owned()
                },
            });
        }

        let bytes = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            res = response.bytes() => res?,
        };

        if bytes.is_empty() {
            return Err(Error::Decode("the download returned no bytes".to_owned()));
        }

        Ok(bytes.to_vec())
    }

    async fn send<T: serde::Serialize>(
        &self,
        path: &str,
        body: &T,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, Error> {
        let request = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .timeout(timeout)
            .json(body);

        // SPEC §3.3: cancellation is a dropped request, not a poll loop.
        // Dropping the future here aborts the connection.
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            res = request.send() => res?,
        };

        let status = response.status();

        let bytes = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            res = response.bytes() => res?,
        };

        // A non-2xx never carries a usable `base_resp`; surface it as-is rather
        // than letting the parser report a shape error.
        if !status.is_success() {
            return Err(Error::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).chars().take(500).collect(),
            });
        }

        Ok(bytes.to_vec())
    }
}
