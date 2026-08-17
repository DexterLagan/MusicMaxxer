//! Response parsing. SPEC §3.1–§3.3.
//!
//! Parsing is deliberately separated from transport so every shape in this file
//! can be tested against recorded fixtures with no network (SPEC §10 step 1).

use crate::error::{user_message, Error};
use crate::request::OutputFormat;
use serde::{Deserialize, Serialize};

/// SPEC §3.3: HTTP 200 does not mean success. Check this before `data`.
#[derive(Debug, Clone, Deserialize)]
pub struct BaseResp {
    pub status_code: i64,
    #[serde(default)]
    pub status_msg: String,
}

/// SPEC §3.1 `data.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    InProgress,
    Completed,
    /// Undocumented value — surfaced rather than guessed at.
    Other(i64),
}

impl JobStatus {
    fn from_wire(v: i64) -> Self {
        match v {
            1 => JobStatus::InProgress,
            2 => JobStatus::Completed,
            other => JobStatus::Other(other),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtraInfo {
    /// SPEC §3.1: milliseconds, not seconds.
    pub music_duration: i64,
    pub music_sample_rate: i64,
    pub music_channel: i64,
    pub bitrate: i64,
    pub music_size: i64,
}

impl ExtraInfo {
    pub fn duration_secs(&self) -> f64 {
        self.music_duration as f64 / 1000.0
    }
}

/// What came back in `data.audio`, resolved against what we asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Audio {
    /// SPEC §3.1: expires after 24 hours. SPEC §6.4: download it immediately.
    Url(String),
    /// Decoded from the hex string.
    Bytes(Vec<u8>),
}

impl Audio {
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Audio::Url(u) => Some(u),
            Audio::Bytes(_) => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Audio::Bytes(b) => Some(b),
            Audio::Url(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Generation {
    pub audio: Audio,
    pub status: JobStatus,
    pub extra: ExtraInfo,
    pub trace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GenerationEnvelope {
    #[serde(default)]
    data: Option<GenerationData>,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    extra_info: Option<ExtraInfo>,
    base_resp: BaseResp,
}

#[derive(Debug, Deserialize)]
struct GenerationData {
    #[serde(default)]
    audio: Option<String>,
    #[serde(default)]
    status: i64,
}

/// Parse a `/v1/music_generation` body.
///
/// `requested` decides how `data.audio` is read: SPEC §3.1 says the same field
/// carries a URL or a hex blob depending on what was asked for, and the body
/// itself gives no way to tell.
pub fn parse_generation(body: &[u8], requested: OutputFormat) -> Result<Generation, Error> {
    let env: GenerationEnvelope = serde_json::from_slice(body)
        .map_err(|e| Error::Decode(format!("unexpected generation response shape: {e}")))?;

    check(&env.base_resp, env.trace_id.as_deref())?;

    let data = env
        .data
        .ok_or_else(|| Error::Decode("success response carried no `data`".to_owned()))?;

    let raw = data
        .audio
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Decode("success response carried no `data.audio`".to_owned()))?;

    let audio = match requested {
        OutputFormat::Url => Audio::Url(raw),
        OutputFormat::Hex => Audio::Bytes(decode_hex(&raw)?),
    };

    Ok(Generation {
        audio,
        status: JobStatus::from_wire(data.status),
        extra: env.extra_info.unwrap_or_default(),
        trace_id: env.trace_id,
    })
}

#[derive(Debug, Clone)]
pub struct CoverPreprocess {
    /// SPEC §3.2: valid 24 hours.
    pub cover_feature_id: String,
    /// ASR-extracted lyrics, already carrying section tags.
    pub formatted_lyrics: String,
    /// SPEC §3.2: `structure_result` arrives as a JSON *string*, so it is
    /// parsed here rather than assumed to be an object.
    ///
    /// TODO/verify: the inner shape (segment field names, whether timestamps
    /// are seconds or milliseconds) is not in the API reference. Left as
    /// `Value` on purpose — type it only after seeing a live response.
    pub structure: serde_json::Value,
    /// The unparsed string, kept verbatim for `run.json`.
    pub structure_raw: String,
    /// SPEC §3.2: seconds, unlike `music_duration`.
    pub audio_duration: f64,
    pub trace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CoverEnvelope {
    #[serde(default)]
    cover_feature_id: Option<String>,
    #[serde(default)]
    formatted_lyrics: Option<String>,
    #[serde(default)]
    structure_result: Option<String>,
    #[serde(default)]
    audio_duration: Option<f64>,
    #[serde(default)]
    trace_id: Option<String>,
    base_resp: BaseResp,
}

/// Parse a `/v1/music_cover_preprocess` body. SPEC §3.2.
pub fn parse_cover_preprocess(body: &[u8]) -> Result<CoverPreprocess, Error> {
    let env: CoverEnvelope = serde_json::from_slice(body)
        .map_err(|e| Error::Decode(format!("unexpected preprocess response shape: {e}")))?;

    check(&env.base_resp, env.trace_id.as_deref())?;

    let cover_feature_id = env
        .cover_feature_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Decode("preprocess returned no `cover_feature_id`".to_owned()))?;

    let structure_raw = env.structure_result.unwrap_or_default();
    let structure = if structure_raw.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&structure_raw)
            .map_err(|e| Error::Decode(format!("`structure_result` was not parseable JSON: {e}")))?
    };

    Ok(CoverPreprocess {
        cover_feature_id,
        formatted_lyrics: env.formatted_lyrics.unwrap_or_default(),
        structure,
        structure_raw,
        audio_duration: env.audio_duration.unwrap_or_default(),
        trace_id: env.trace_id,
    })
}

/// SPEC §3.3: the single gate every response goes through.
fn check(base: &BaseResp, trace_id: Option<&str>) -> Result<(), Error> {
    if base.status_code == 0 {
        return Ok(());
    }
    Err(Error::Api {
        code: base.status_code,
        message: user_message(base.status_code, &base.status_msg),
        status_msg: base.status_msg.clone(),
        trace_id: trace_id.map(str::to_owned),
    })
}

fn decode_hex(s: &str) -> Result<Vec<u8>, Error> {
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(Error::Decode(
            "hex audio payload had an odd number of digits".to_owned(),
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = hex_digit(pair[0])?;
        let lo = hex_digit(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_digit(b: u8) -> Result<u8, Error> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        other => Err(Error::Decode(format!(
            "hex audio payload contained `{}`",
            other as char
        ))),
    }
}
