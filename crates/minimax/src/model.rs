//! Model IDs. SPEC §3.1 — these six and their RPM figures came from the
//! official reference. Do not add plausible-looking variants.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Model {
    #[serde(rename = "music-3.0")]
    Music30,
    #[serde(rename = "music-3.0-free")]
    Music30Free,
    #[serde(rename = "music-2.6")]
    Music26,
    #[serde(rename = "music-2.6-free")]
    Music26Free,
    #[serde(rename = "music-cover")]
    MusicCover,
    #[serde(rename = "music-cover-free")]
    MusicCoverFree,
}

impl Model {
    pub const ALL: [Model; 6] = [
        Model::Music30,
        Model::Music30Free,
        Model::Music26,
        Model::Music26Free,
        Model::MusicCover,
        Model::MusicCoverFree,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Model::Music30 => "music-3.0",
            Model::Music30Free => "music-3.0-free",
            Model::Music26 => "music-2.6",
            Model::Music26Free => "music-2.6-free",
            Model::MusicCover => "music-cover",
            Model::MusicCoverFree => "music-cover-free",
        }
    }

    /// Requests per minute. Drives the client-side limiter (SPEC §4).
    pub fn rpm(self) -> u32 {
        if self.is_free_tier() {
            3
        } else {
            120
        }
    }

    /// `-free` variants work with any API key; the others need a funded
    /// Token Plan account and answer `1008` without one.
    pub fn is_free_tier(self) -> bool {
        matches!(
            self,
            Model::Music30Free | Model::Music26Free | Model::MusicCoverFree
        )
    }

    /// Cover models are the only ones that accept `audio_url`,
    /// `audio_base64` or `cover_feature_id`.
    pub fn is_cover(self) -> bool {
        matches!(self, Model::MusicCover | Model::MusicCoverFree)
    }

    /// The model to suggest after a `1008`.
    pub fn free_equivalent(self) -> Model {
        match self {
            Model::Music30 => Model::Music30Free,
            Model::Music26 => Model::Music26Free,
            Model::MusicCover => Model::MusicCoverFree,
            already_free => already_free,
        }
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Model {
    type Err = UnknownModel;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Model::ALL
            .into_iter()
            .find(|m| m.as_str() == s)
            .ok_or_else(|| UnknownModel(s.to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown model id `{0}`")]
pub struct UnknownModel(pub String);
