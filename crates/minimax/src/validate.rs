//! Client-side validation. SPEC §4.
//!
//! Validate before sending: a rejected request still costs a round trip and,
//! on the free tier, one of three requests per minute.

use crate::model::Model;
use crate::request::GenerationRequest;
use std::fmt;

/// Limits from SPEC §3.1. Character counts, not bytes — the UI counters must
/// agree with these or the button state will lie.
pub mod limits {
    pub const PROMPT_MAX: usize = 2000;
    pub const LYRICS_MIN: usize = 1;
    pub const LYRICS_MAX: usize = 3500;

    pub const COVER_PROMPT_MIN: usize = 10;
    pub const COVER_PROMPT_MAX: usize = 300;
    pub const COVER_LYRICS_MIN: usize = 10;
    pub const COVER_LYRICS_MAX: usize = 1000;

    /// SPEC §3.3, reference audio.
    pub const REF_MIN_SECS: f64 = 6.0;
    pub const REF_MAX_SECS: f64 = 360.0;
    pub const REF_MAX_BYTES: u64 = 50 * 1024 * 1024;
}

/// Which control to mark, so the UI does not have to parse the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Model,
    Prompt,
    Lyrics,
    ReferenceAudio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub field: Field,
    pub message: String,
}

/// The issues found in one request. Empty means it is safe to send.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report(pub Vec<Issue>);

impl Report {
    pub fn is_ok(&self) -> bool {
        self.0.is_empty()
    }

    pub fn for_field(&self, f: Field) -> impl Iterator<Item = &Issue> {
        self.0.iter().filter(move |i| i.field == f)
    }

    fn push(&mut self, field: Field, message: impl Into<String>) {
        self.0.push(Issue {
            field,
            message: message.into(),
        });
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joined: Vec<&str> = self.0.iter().map(|i| i.message.as_str()).collect();
        f.write_str(&joined.join("; "))
    }
}

/// Validate a generation request. SPEC §4.
pub fn generation(req: &GenerationRequest) -> Report {
    let mut r = Report::default();
    let cover = req.model.is_cover();

    let prompt_len = req.prompt.as_deref().map(count).unwrap_or(0);
    let lyrics_len = req.lyrics.as_deref().map(count).unwrap_or(0);

    if cover {
        cover_rules(req, &mut r, prompt_len, lyrics_len);
    } else {
        original_rules(req, &mut r, prompt_len, lyrics_len);
    }

    r
}

fn original_rules(req: &GenerationRequest, r: &mut Report, prompt_len: usize, lyrics_len: usize) {
    if prompt_len > limits::PROMPT_MAX {
        r.push(
            Field::Prompt,
            format!(
                "Caption is {prompt_len} characters — the limit is {}",
                limits::PROMPT_MAX
            ),
        );
    }

    if lyrics_len > limits::LYRICS_MAX {
        r.push(
            Field::Lyrics,
            format!(
                "Lyrics are {lyrics_len} characters — the limit is {}",
                limits::LYRICS_MAX
            ),
        );
    }

    // SPEC §3.1: prompt required when instrumental; lyrics required otherwise,
    // unless the model is writing them.
    if req.is_instrumental {
        if prompt_len == 0 {
            r.push(
                Field::Prompt,
                "An instrumental needs a caption describing the music",
            );
        }
    } else if lyrics_len < limits::LYRICS_MIN && !req.lyrics_optimizer {
        r.push(
            Field::Lyrics,
            "Add lyrics, switch on Instrumental, or let the model write them",
        );
    }

    if req.lyrics_optimizer && lyrics_len > 0 {
        r.push(
            Field::Lyrics,
            "Auto-write only applies when the lyrics field is empty",
        );
    }

    if req.is_instrumental && req.lyrics_optimizer {
        r.push(
            Field::Lyrics,
            "Instrumental and auto-write cannot both be on",
        );
    }

    // Cover-only fields on a non-cover model.
    if req.audio_url.is_some() || req.audio_base64.is_some() || req.cover_feature_id.is_some() {
        r.push(
            Field::Model,
            format!(
                "{} is not a cover model, so it cannot take reference audio",
                req.model
            ),
        );
    }
}

fn cover_rules(req: &GenerationRequest, r: &mut Report, prompt_len: usize, lyrics_len: usize) {
    if !(limits::COVER_PROMPT_MIN..=limits::COVER_PROMPT_MAX).contains(&prompt_len) {
        r.push(
            Field::Prompt,
            format!(
                "A cover's style prompt must be {}–{} characters — this is {prompt_len}",
                limits::COVER_PROMPT_MIN,
                limits::COVER_PROMPT_MAX
            ),
        );
    }

    // SPEC §3.1: optional for covers, but bounded when present.
    if lyrics_len > 0
        && !(limits::COVER_LYRICS_MIN..=limits::COVER_LYRICS_MAX).contains(&lyrics_len)
    {
        r.push(
            Field::Lyrics,
            format!(
                "A cover's lyrics must be {}–{} characters — this is {lyrics_len}",
                limits::COVER_LYRICS_MIN,
                limits::COVER_LYRICS_MAX
            ),
        );
    }

    // SPEC §4: exactly one of the three.
    let sources = [
        req.audio_url.is_some(),
        req.audio_base64.is_some(),
        req.cover_feature_id.is_some(),
    ];
    match sources.iter().filter(|set| **set).count() {
        0 => r.push(
            Field::ReferenceAudio,
            "A cover needs reference audio: a file, a URL, or a preprocessed feature ID",
        ),
        1 => {}
        _ => r.push(
            Field::ReferenceAudio,
            "Set only one of reference file, reference URL, or feature ID",
        ),
    }

    // SPEC §3.1: with cover_feature_id, lyrics are required.
    if req.cover_feature_id.is_some() && lyrics_len == 0 {
        r.push(
            Field::Lyrics,
            "The two-step cover flow requires lyrics — edit the extracted ones or write your own",
        );
    }
}

/// Validate reference audio before upload. SPEC §3.3, §4 — the message carries
/// the measured value so the user knows how far off they are.
pub fn reference_audio(duration_secs: f64, size_bytes: u64) -> Report {
    let mut r = Report::default();

    if duration_secs < limits::REF_MIN_SECS {
        r.push(
            Field::ReferenceAudio,
            format!(
                "Reference audio is {duration_secs:.1}s — the minimum is {:.0}s",
                limits::REF_MIN_SECS
            ),
        );
    } else if duration_secs > limits::REF_MAX_SECS {
        r.push(
            Field::ReferenceAudio,
            format!(
                "Reference audio is {:.1} minutes — the maximum is 6 minutes",
                duration_secs / 60.0
            ),
        );
    }

    if size_bytes > limits::REF_MAX_BYTES {
        r.push(
            Field::ReferenceAudio,
            format!(
                "Reference audio is {:.1} MB — the maximum is 50 MB",
                size_bytes as f64 / (1024.0 * 1024.0)
            ),
        );
    }

    r
}

/// SPEC §3.2: preprocess takes only the cover models.
pub fn preprocess_model(model: Model) -> Report {
    let mut r = Report::default();
    if !model.is_cover() {
        r.push(
            Field::Model,
            format!("{model} cannot preprocess reference audio — use a cover model"),
        );
    }
    r
}

/// Characters, matching what a UI counter shows. Not bytes.
fn count(s: &str) -> usize {
    s.chars().count()
}
