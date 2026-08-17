//! The library on disk. SPEC §5.
//!
//! One folder per song, takes inside it, and three kinds of file with one job
//! each: `song.md` the recipe (§7, not yet written here), `run.json` the
//! immutable receipt, `meta.json` the mutable annotation.
//!
//! Everything here is path arithmetic and file I/O against a root the caller
//! supplies, so the whole module is testable against a temporary directory.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("Could not write to the library: {0}")]
    Io(String),
    #[error("{0} is outside the library folder")]
    OutsideRoot(String),
    #[error("Could not read {what}: {why}")]
    Malformed { what: String, why: String },
}

fn io(e: std::io::Error) -> LibraryError {
    LibraryError::Io(e.to_string())
}

// --------------------------------------------------------------- the records

/// The immutable receipt. SPEC §5.2 — written once, never hand-edited.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub title: String,
    pub model: String,
    pub caption: String,
    #[serde(default)]
    pub lyrics: String,
    pub instrumental: bool,
    /// The complete request body as sent, so a take can be reproduced exactly.
    pub request: serde_json::Value,
    pub extra_info: serde_json::Value,
    pub trace_id: Option<String>,
    /// ISO-8601, so the file is readable without the app.
    pub created_at: String,
    /// Wall-clock seconds the call took — the only honest guide to how long
    /// the next one will take (SPEC §3.5).
    pub call_secs: f64,
    pub audio_file: String,
    pub duration_secs: f64,
    pub bytes: u64,
    /// SPEC §6.4: AI disclosure travels with the take, not just the tags.
    #[serde(default = "ai_note")]
    pub generated_by_ai: String,
}

fn ai_note() -> String {
    "Generated with MiniMax music models via the MiniMax API.".to_owned()
}

/// Song-level bookkeeping, at the song folder root. SPEC §5.1.
///
/// Exists for two reasons that both bite without it:
///
/// - `next_take` is a **high-water mark**. Deriving the next number from the
///   folders present reuses a number as soon as you delete the highest take,
///   which breaks the promise that "take 3" always means the same take.
/// - `title` identifies which song owns this slug even when every take has
///   been deleted. Read from a take's receipt instead and an emptied folder
///   gets silently adopted by a different song with the same slug.
///
/// Machine-owned. `song.md` (§7) is the human-owned file in the same folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongIndex {
    pub title: String,
    pub next_take: u32,
}

/// The mutable annotation. SPEC §5.3.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TakeMeta {
    /// 0 = unrated. Nothing is written until a take is actually rated.
    #[serde(default)]
    pub rating: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rated_at: Option<String>,
}

/// One take, as the history list needs it.
#[derive(Debug, Clone, Serialize)]
pub struct StoredTake {
    pub song_slug: String,
    pub title: String,
    pub take: u32,
    pub dir: String,
    pub audio_path: String,
    pub duration_secs: f64,
    pub model: String,
    pub caption: String,
    pub created_at: String,
    pub call_secs: f64,
    pub rating: u8,
}

/// What `save` needs. Kept separate from `RunRecord` so callers cannot forget
/// a field the receipt requires.
pub struct NewTake<'a> {
    pub title: &'a str,
    pub model: &'a str,
    pub caption: &'a str,
    pub lyrics: &'a str,
    pub instrumental: bool,
    pub lyrics_optimizer: bool,
    pub request: serde_json::Value,
    pub extra_info: serde_json::Value,
    pub trace_id: Option<String>,
    pub call_secs: f64,
    pub duration_secs: f64,
    pub audio: &'a [u8],
    pub extension: &'a str,
    pub sample_rate: u32,
    /// Only meaningful for mp3 (SPEC §3.1).
    pub bitrate: Option<u32>,
}

// ------------------------------------------------------------------- library

pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Library { root: root.into() }
    }

    /// `~/MiniMaxMusic`, falling back to the current directory only if the home
    /// directory cannot be determined.
    pub fn default_root() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("MiniMaxMusic")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Save a take, creating the song folder and the next take folder.
    pub fn save(&self, take: NewTake<'_>) -> Result<StoredTake, LibraryError> {
        std::fs::create_dir_all(&self.root).map_err(io)?;

        let slug = self.slug_for(take.title)?;
        let song_dir = self.root.join(&slug);
        std::fs::create_dir_all(&song_dir).map_err(io)?;

        // Claim the next number from the high-water mark and persist it before
        // writing anything, so a crash mid-save loses a number rather than
        // handing the same one out twice.
        let mut index = read_index(&song_dir).unwrap_or(SongIndex {
            title: display_title(take.title),
            next_take: 1,
        });
        let number = index.next_take;
        index.next_take += 1;
        write_index(&song_dir, &index)?;

        let take_dir = song_dir.join(format!("take-{number:02}"));
        std::fs::create_dir_all(&take_dir).map_err(io)?;

        let audio_file = format!("track.{}", take.extension);
        std::fs::write(take_dir.join(&audio_file), take.audio).map_err(io)?;

        let record = RunRecord {
            title: display_title(take.title),
            model: take.model.to_owned(),
            caption: take.caption.to_owned(),
            lyrics: take.lyrics.to_owned(),
            instrumental: take.instrumental,
            request: take.request,
            extra_info: take.extra_info,
            trace_id: take.trace_id,
            created_at: now_iso(),
            call_secs: take.call_secs,
            audio_file: audio_file.clone(),
            duration_secs: take.duration_secs,
            bytes: take.audio.len() as u64,
            generated_by_ai: ai_note(),
        };

        let json =
            serde_json::to_string_pretty(&record).map_err(|e| LibraryError::Io(e.to_string()))?;
        std::fs::write(take_dir.join("run.json"), json).map_err(io)?;

        // SPEC §7: the recipe sits at the song root and is rewritten on every
        // generate, so it always reflects the inputs that produced the newest
        // take. Failing to write it must not lose the audio we just fetched.
        let recipe = crate::recipe::Recipe {
            title: record.title.clone(),
            created: record.created_at.clone(),
            model: record.model.clone(),
            instrumental: take.instrumental,
            lyrics_optimizer: take.lyrics_optimizer,
            audio: crate::recipe::AudioSettings {
                format: take.extension.to_owned(),
                sample_rate: take.sample_rate,
                bitrate: take.bitrate,
            },
            cover: None,
            caption: take.caption.to_owned(),
            lyrics: take.lyrics.to_owned(),
        };
        if let Err(e) = recipe.write(&song_dir) {
            eprintln!("warning: could not write song.md: {e}");
        }

        Ok(StoredTake {
            song_slug: slug,
            title: record.title,
            take: number,
            dir: take_dir.to_string_lossy().into_owned(),
            audio_path: take_dir.join(&audio_file).to_string_lossy().into_owned(),
            duration_secs: record.duration_secs,
            model: record.model,
            caption: record.caption,
            created_at: record.created_at,
            call_secs: record.call_secs,
            rating: 0,
        })
    }

    /// Every take in the library, newest first.
    pub fn scan(&self) -> Result<Vec<StoredTake>, LibraryError> {
        let mut out = Vec::new();
        let Ok(songs) = std::fs::read_dir(&self.root) else {
            return Ok(out); // an empty library is not an error
        };

        for song in songs.flatten() {
            let song_dir = song.path();
            if !song_dir.is_dir() {
                continue;
            }
            let slug = song.file_name().to_string_lossy().into_owned();

            let Ok(takes) = std::fs::read_dir(&song_dir) else {
                continue;
            };
            for take in takes.flatten() {
                let dir = take.path();
                if !dir.is_dir() {
                    continue;
                }
                let Some(number) = take_number(&dir) else {
                    continue;
                };
                // A folder without a receipt is not a take — a stray directory
                // in the library must not crash the history list.
                let Some(record) = read_run(&dir) else {
                    continue;
                };

                out.push(StoredTake {
                    song_slug: slug.clone(),
                    title: record.title,
                    take: number,
                    audio_path: dir.join(&record.audio_file).to_string_lossy().into_owned(),
                    dir: dir.to_string_lossy().into_owned(),
                    duration_secs: record.duration_secs,
                    model: record.model,
                    caption: record.caption,
                    created_at: record.created_at,
                    call_secs: record.call_secs,
                    rating: read_meta(&dir).rating.min(5),
                });
            }
        }

        // Newest first, with take number breaking ties so a re-import with
        // identical timestamps still reads in a sensible order.
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.take.cmp(&a.take)));
        Ok(out)
    }

    /// SPEC §5.3. `stars` is clamped to 0–5; 0 clears the rating.
    pub fn set_rating(&self, take_dir: &str, stars: u8) -> Result<(), LibraryError> {
        let dir = self.inside(take_dir)?;
        let stars = stars.min(5);

        let meta = TakeMeta {
            rating: stars,
            rated_at: (stars > 0).then(now_iso),
        };
        let json =
            serde_json::to_string_pretty(&meta).map_err(|e| LibraryError::Io(e.to_string()))?;
        std::fs::write(dir.join("meta.json"), json).map_err(io)
    }

    /// SPEC §5.4: deletion only ever removes the app's own directory.
    pub fn delete_take(&self, take_dir: &str) -> Result<(), LibraryError> {
        let dir = self.inside(take_dir)?;
        std::fs::remove_dir_all(&dir).map_err(io)?;

        // Deleting the last take deletes the song: what remains is bookkeeping
        // (`song.json`, later `song.md`) for something with no recordings. The
        // high-water mark goes with it, which is right — the song is gone, so
        // a future song of the same name starts at take-01.
        // `dir` came back canonicalised from `inside`, so the root must be too
        // before they are compared — on macOS `/var` resolves to `/private/var`
        // and a raw comparison silently never matches.
        let root = self.canonical_root()?;
        if let Some(song_dir) = dir.parent() {
            if song_dir != root && song_dir.starts_with(&root) && !has_takes(song_dir) {
                let _ = std::fs::remove_dir_all(song_dir);
            }
        }
        Ok(())
    }

    /// The song folder for a title, suffixing when a *different* title already
    /// owns the slug. SPEC §5.1.
    fn slug_for(&self, title: &str) -> Result<String, LibraryError> {
        let base = slugify(&display_title(title));
        let wanted = display_title(title);

        for attempt in 1..=99u32 {
            let candidate = if attempt == 1 {
                base.clone()
            } else {
                format!("{base}-{attempt}")
            };
            let dir = self.root.join(&candidate);

            if !dir.exists() {
                return Ok(candidate);
            }
            match read_index(&dir).map(|i| i.title) {
                // No index yet — an empty folder we can take over.
                None => return Ok(candidate),
                Some(t) if t == wanted => return Ok(candidate),
                Some(_) => continue, // a different song owns this slug
            }
        }
        Err(LibraryError::Io(format!(
            "too many songs share the name {wanted}"
        )))
    }

    /// A take's own settings, reconstructed from its immutable `run.json` —
    /// used when the history rail loads a take back into the compose form
    /// (SPEC §7.3).
    ///
    /// This must not read the song's `song.md`: that file is rewritten on
    /// every generate (SPEC §7) and always reflects only the newest take, so
    /// clicking an older take would silently load a different take's caption
    /// and lyrics. `run.json`'s `request` field is documented as "the
    /// complete request body as sent, so a take can be reproduced exactly" —
    /// that is precisely this job.
    ///
    /// The take dir is checked against the library root the same way
    /// deletion is, so a path from the webview cannot walk anywhere else.
    pub fn recipe_from_take(&self, take_dir: &str) -> Result<crate::recipe::Recipe, LibraryError> {
        let dir = self.inside(take_dir)?;
        let raw = std::fs::read_to_string(dir.join("run.json")).map_err(io)?;
        let record: RunRecord =
            serde_json::from_str(&raw).map_err(|e| LibraryError::Malformed {
                what: "run.json".to_owned(),
                why: e.to_string(),
            })?;

        let audio_setting = record.request.get("audio_setting");
        let format = audio_setting
            .and_then(|a| a.get("format"))
            .and_then(|v| v.as_str())
            .unwrap_or("wav")
            .to_owned();
        let sample_rate = audio_setting
            .and_then(|a| a.get("sample_rate"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(44100);
        let bitrate = audio_setting
            .and_then(|a| a.get("bitrate"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let lyrics_optimizer = record
            .request
            .get("lyrics_optimizer")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        Ok(crate::recipe::Recipe {
            title: record.title,
            created: record.created_at,
            model: record.model,
            instrumental: record.instrumental,
            lyrics_optimizer,
            audio: crate::recipe::AudioSettings {
                format,
                sample_rate,
                bitrate,
            },
            // Cover recipes are not tracked per-take yet — the Cover tab
            // does not exist, so nothing writes a reference here.
            cover: None,
            caption: record.caption,
            lyrics: record.lyrics,
        })
    }

    fn canonical_root(&self) -> Result<PathBuf, LibraryError> {
        self.root
            .canonicalize()
            .map_err(|_| LibraryError::OutsideRoot(self.root.to_string_lossy().into_owned()))
    }

    /// Refuse any path that is not under the library root. SPEC §5.4 —
    /// "never touch files outside the library root."
    fn inside(&self, path: &str) -> Result<PathBuf, LibraryError> {
        let candidate = PathBuf::from(path);
        // Canonicalise both sides so `..` cannot walk out, and so a symlinked
        // root (macOS `/var`) compares against a canonical child correctly.
        let root = self.canonical_root()?;
        let resolved = candidate
            .canonicalize()
            .map_err(|_| LibraryError::OutsideRoot(path.to_owned()))?;

        if resolved.starts_with(&root) && resolved != root {
            Ok(resolved)
        } else {
            Err(LibraryError::OutsideRoot(path.to_owned()))
        }
    }
}

// ------------------------------------------------------------------- helpers

pub fn display_title(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        "Untitled song".to_owned()
    } else {
        t.to_owned()
    }
}

/// SPEC §5.1: lowercase, non-alphanumerics to single hyphens, trimmed, ≤ 40.
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut last_hyphen = false;

    for ch in title.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_hyphen = false;
        } else if !last_hyphen && !out.is_empty() {
            out.push('-');
            last_hyphen = true;
        }
    }

    let trimmed = out.trim_matches('-');
    let capped: String = trimmed.chars().take(40).collect();
    let capped = capped.trim_matches('-').to_owned();

    if capped.is_empty() {
        "untitled-song".to_owned()
    } else {
        capped
    }
}

fn has_takes(song_dir: &Path) -> bool {
    std::fs::read_dir(song_dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| take_number(&e.path()).is_some() && e.path().is_dir())
}

fn read_index(song_dir: &Path) -> Option<SongIndex> {
    let raw = std::fs::read_to_string(song_dir.join("song.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_index(song_dir: &Path, index: &SongIndex) -> Result<(), LibraryError> {
    let json = serde_json::to_string_pretty(index).map_err(|e| LibraryError::Io(e.to_string()))?;
    std::fs::write(song_dir.join("song.json"), json).map_err(io)
}

fn take_number(dir: &Path) -> Option<u32> {
    dir.file_name()?
        .to_str()?
        .strip_prefix("take-")?
        .parse()
        .ok()
}

fn read_run(take_dir: &Path) -> Option<RunRecord> {
    let raw = std::fs::read_to_string(take_dir.join("run.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

fn read_meta(take_dir: &Path) -> TakeMeta {
    std::fs::read_to_string(take_dir.join("meta.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned())
}
