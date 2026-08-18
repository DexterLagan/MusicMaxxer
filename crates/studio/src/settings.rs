//! Persisted output settings: model, format/sample-rate/bitrate, and an
//! optional library location override. SPEC §6.3.
//!
//! Not sensitive, so this is a plain JSON file rather than the keychain —
//! Roaming AppData on Windows, Application Support on macOS, the OS
//! convention for small app config. Deliberately a *different* folder from
//! the library itself: settings must be findable before we know where the
//! library is, since the library root is one of the settings.
//!
//! The fields reuse `minimax`'s closed enums directly rather than re-typing
//! validation here. A hand-edited `sample_rate: 99999` in the file on disk
//! simply fails to deserialize, and `load()` falls back to defaults —
//! consistent with every other file this app reads back (`song.md`,
//! `run.json`): a malformed file degrades to "use sane defaults", not a
//! crash.

use minimax::{AudioFormat, AudioSetting, Bitrate, Model, SampleRate};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub model: Model,
    pub format: AudioFormat,
    pub sample_rate: SampleRate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<Bitrate>,
    /// `None` means "use the platform default" — see `Library::default_root`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_root: Option<PathBuf>,
}

impl Default for Settings {
    /// SPEC §6.3: default to wav @ 44100 — the user is producing masters,
    /// not previews. Matches `AudioSetting::masters()`.
    fn default() -> Self {
        Settings {
            model: Model::Music30Free,
            format: AudioFormat::Wav,
            sample_rate: SampleRate::Hz44100,
            bitrate: None,
            library_root: None,
        }
    }
}

impl Settings {
    /// `%APPDATA%\MusicMaxxer\settings.json` on Windows,
    /// `~/Library/Application Support/MusicMaxxer/settings.json` on macOS.
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MusicMaxxer")
            .join("settings.json")
    }

    /// Never fails outward: a missing or corrupt file just means defaults.
    pub fn load() -> Settings {
        Self::load_from(&Self::config_path())
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::config_path())
    }

    /// Split out from `load`/`save` so tests can point at a temp file instead
    /// of the real per-OS config path — the same shape as `Library::new`
    /// taking an explicit root rather than always resolving one itself.
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// The library folder actually in effect right now.
    pub fn effective_library_root(&self) -> PathBuf {
        self.library_root
            .clone()
            .unwrap_or_else(crate::library::Library::default_root)
    }

    /// Drops the bitrate when the format ignores it (SPEC §3.1: mp3 only),
    /// so a stale mp3 bitrate can't ride along on a wav/pcm setting. Reuses
    /// `AudioSetting::new`'s rule rather than restating it.
    pub fn normalise(&mut self) {
        self.bitrate = AudioSetting::new(self.format, self.sample_rate, self.bitrate).bitrate;
    }
}
