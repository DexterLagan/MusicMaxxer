//! Request bodies. SPEC §3.1 and §3.2.
//!
//! The enums here are closed on purpose: the sample rates and bitrates are the
//! documented set, and an arbitrary integer is not a safe substitute.

use crate::model::Model;
use serde::de::{Error as DeError, Unexpected};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// SPEC §3.1: the server default is `hex`. Always set this explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Url,
    Hex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Mp3,
    Wav,
    Pcm,
}

impl AudioFormat {
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Wav => "wav",
            AudioFormat::Pcm => "pcm",
        }
    }

    /// SPEC §6.3: bitrate is mp3-only, and so is recipe tag embedding.
    pub fn supports_bitrate(self) -> bool {
        matches!(self, AudioFormat::Mp3)
    }
}

macro_rules! int_enum {
    ($name:ident, $doc:literal, { $($variant:ident => $value:expr),+ $(,)? }) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn value(self) -> u32 {
                match self { $($name::$variant => $value),+ }
            }

            pub fn from_value(v: u32) -> Option<Self> {
                match v { $($value => Some($name::$variant),)+ _ => None }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_u32(self.value())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let v = u32::deserialize(d)?;
                $name::from_value(v).ok_or_else(|| {
                    D::Error::invalid_value(
                        Unexpected::Unsigned(v as u64),
                        &concat!("one of the documented ", stringify!($name), " values"),
                    )
                })
            }
        }
    };
}

int_enum!(SampleRate, "SPEC §3.1 `audio_setting.sample_rate`.", {
    Hz16000 => 16000,
    Hz24000 => 24000,
    Hz32000 => 32000,
    Hz44100 => 44100,
});

int_enum!(Bitrate, "SPEC §3.1 `audio_setting.bitrate`. mp3 only; inert otherwise.", {
    Kbps32 => 32000,
    Kbps64 => 64000,
    Kbps128 => 128000,
    Kbps256 => 256000,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSetting {
    pub sample_rate: SampleRate,
    /// Omitted for wav and pcm, where it has no effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<Bitrate>,
    pub format: AudioFormat,
}

impl AudioSetting {
    /// SPEC §6.3: default to wav @ 44100 — the user is producing masters.
    pub fn masters() -> Self {
        AudioSetting {
            sample_rate: SampleRate::Hz44100,
            bitrate: None,
            format: AudioFormat::Wav,
        }
    }

    /// Builds a setting with the bitrate dropped when the format ignores it,
    /// so a stale mp3 bitrate can't ride along on a wav request.
    pub fn new(format: AudioFormat, sample_rate: SampleRate, bitrate: Option<Bitrate>) -> Self {
        AudioSetting {
            sample_rate,
            bitrate: if format.supports_bitrate() {
                bitrate
            } else {
                None
            },
            format,
        }
    }
}

/// `POST /v1/music_generation`. SPEC §3.1.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenerationRequest {
    pub model: Model,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<String>,

    /// SPEC §3.1: streaming is out of scope. Always false.
    pub stream: bool,

    pub output_format: OutputFormat,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_setting: Option<AudioSetting>,

    pub lyrics_optimizer: bool,
    pub is_instrumental: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_feature_id: Option<String>,
}

impl GenerationRequest {
    /// A request with the non-negotiable fields already set: `stream: false`,
    /// and an explicit `output_format` (SPEC §6.4 wants `Url` in the app).
    pub fn new(model: Model, output_format: OutputFormat) -> Self {
        GenerationRequest {
            model,
            prompt: None,
            lyrics: None,
            stream: false,
            output_format,
            audio_setting: Some(AudioSetting::masters()),
            lyrics_optimizer: false,
            is_instrumental: false,
            audio_url: None,
            audio_base64: None,
            cover_feature_id: None,
        }
    }

    pub fn prompt(mut self, s: impl Into<String>) -> Self {
        self.prompt = Some(s.into());
        self
    }

    pub fn lyrics(mut self, s: impl Into<String>) -> Self {
        self.lyrics = Some(s.into());
        self
    }

    pub fn audio_setting(mut self, a: AudioSetting) -> Self {
        self.audio_setting = Some(a);
        self
    }

    pub fn instrumental(mut self, yes: bool) -> Self {
        self.is_instrumental = yes;
        self
    }

    /// SPEC §3.5: the words are never returned. The caller owes the user an
    /// ASR recovery pass after the take lands.
    pub fn lyrics_optimizer(mut self, yes: bool) -> Self {
        self.lyrics_optimizer = yes;
        self
    }

    pub fn cover_feature_id(mut self, id: impl Into<String>) -> Self {
        self.cover_feature_id = Some(id.into());
        self
    }

    pub fn audio_url(mut self, url: impl Into<String>) -> Self {
        self.audio_url = Some(url.into());
        self
    }

    pub fn audio_base64(mut self, b64: impl Into<String>) -> Self {
        self.audio_base64 = Some(b64.into());
        self
    }
}

/// Exactly one of these may be set on a cover request. SPEC §3.1, §4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceAudio {
    Url(String),
    Base64(String),
}

/// `POST /v1/music_cover_preprocess`. SPEC §3.2. Free, and deduplicated by
/// content MD5 server-side — cache by content hash and skip repeat calls.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverPreprocessRequest {
    pub model: Model,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_base64: Option<String>,
}

impl CoverPreprocessRequest {
    pub fn new(model: Model, reference: ReferenceAudio) -> Self {
        let (audio_url, audio_base64) = match reference {
            ReferenceAudio::Url(u) => (Some(u), None),
            ReferenceAudio::Base64(b) => (None, Some(b)),
        };
        CoverPreprocessRequest {
            model,
            audio_url,
            audio_base64,
        }
    }
}
