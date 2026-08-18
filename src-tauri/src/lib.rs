//! Tauri commands — the only bridge between the webview and everything else.
//!
//! SPEC §2: all HTTP happens here, in Rust. The webview cannot read the API
//! key, cannot reach the network, and has no filesystem permission (see
//! `capabilities/default.json`).
//!
//! The one place a key crosses the boundary is `save_key`, when the user types
//! it into the settings field. That direction is unavoidable in a GUI — what
//! matters is that it is write-only: no command ever hands a key back out.

use minimax::{
    AudioFormat, AudioSetting, Bitrate, Client, Error, GenerationRequest, Model, OutputFormat,
    SampleRate,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use studio::keys::suspicious;
use studio::library::NewTake;
use studio::{Credential, Keychain, Library, Recipe, SecretStore, Settings, StoredTake};
use tokio_util::sync::CancellationToken;

pub struct AppState {
    store: Keychain,
    settings: Mutex<Settings>,
    /// The in-flight generation, so Cancel has something to pull.
    current: Mutex<Option<CancellationToken>>,
}

impl AppState {
    fn new() -> Self {
        AppState {
            store: Keychain,
            settings: Mutex::new(Settings::load()),
            current: Mutex::new(None),
        }
    }

    /// A fresh `Library` pointed at whatever root is currently in effect.
    /// `Library` is a thin path wrapper, so building one per call is cheap —
    /// far cheaper than a caching scheme that could go stale the moment the
    /// library location setting changes at runtime.
    fn library(&self) -> Library {
        Library::new(self.settings.lock().unwrap().effective_library_root())
    }
}

// ------------------------------------------------------------------ payloads

#[derive(Debug, Serialize)]
pub struct KeyStatus {
    /// Whether a key is stored. Never the key itself.
    pub minimax: bool,
}

#[derive(Debug, Deserialize)]
pub struct ComposeRequest {
    pub title: String,
    pub caption: String,
    pub lyrics: String,
    pub instrumental: bool,
    // Typed directly rather than as strings: an invalid model id or sample
    // rate fails Tauri's own IPC deserialization before this command's body
    // ever runs, the same free validation `save_settings` gets below.
    pub model: Model,
    pub format: AudioFormat,
    pub sample_rate: SampleRate,
    pub bitrate: Option<Bitrate>,
}

#[derive(Debug, Serialize)]
pub struct TakeResult {
    /// The stored take, in the same shape the history list uses, so the UI can
    /// insert it into the rail without a re-scan.
    #[serde(flatten)]
    pub take: StoredTake,
    pub trace_id: Option<String>,
    pub bytes: u64,
}

/// What the UI renders when something goes wrong. `message` is already the
/// user-facing sentence from SPEC §3.4; `detail` is for the expander.
#[derive(Debug, Serialize)]
pub struct Failure {
    pub message: String,
    pub code: Option<i64>,
    pub trace_id: Option<String>,
    pub cancelled: bool,
    /// Per-field validation issues, so the UI can mark the right control.
    pub fields: Vec<FieldIssue>,
}

#[derive(Debug, Serialize)]
pub struct FieldIssue {
    pub field: String,
    pub message: String,
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        let fields = match &e {
            Error::Invalid(report) => report
                .0
                .iter()
                .map(|i| FieldIssue {
                    field: match i.field {
                        minimax::Field::Model => "model",
                        minimax::Field::Prompt => "caption",
                        minimax::Field::Lyrics => "lyrics",
                        minimax::Field::ReferenceAudio => "reference",
                    }
                    .to_owned(),
                    message: i.message.clone(),
                })
                .collect(),
            _ => Vec::new(),
        };

        Failure {
            message: e.to_string(),
            code: e.code(),
            trace_id: e.trace_id().map(str::to_owned),
            cancelled: matches!(e, Error::Cancelled),
            fields,
        }
    }
}

/// One selectable model, with the RPM figure the UI shows inline (SPEC §6.3)
/// so the free/paid tradeoff is visible before it becomes a 1008.
#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub rpm: u32,
    pub free_tier: bool,
}

/// The settings panel's whole state in one payload, so the UI can populate
/// every control from a single call at boot.
#[derive(Debug, Serialize)]
pub struct SettingsPayload {
    pub model: String,
    pub format: String,
    pub sample_rate: u32,
    pub bitrate: Option<u32>,
    /// The folder actually in effect right now — resolved server-side so the
    /// UI never has to know the per-OS default rule.
    pub library_root: String,
    /// Whether `library_root` above is a user override or the platform
    /// default, so the UI knows whether "Reset to default" has anything to do.
    pub is_custom_library_root: bool,
}

fn settings_payload(s: &Settings) -> SettingsPayload {
    SettingsPayload {
        model: s.model.as_str().to_owned(),
        format: s.format.extension().to_owned(),
        sample_rate: s.sample_rate.value(),
        bitrate: s.bitrate.map(Bitrate::value),
        library_root: s.effective_library_root().to_string_lossy().into_owned(),
        is_custom_library_root: s.library_root.is_some(),
    }
}

// ------------------------------------------------------------------ commands

#[tauri::command]
fn key_status(state: tauri::State<'_, AppState>) -> KeyStatus {
    KeyStatus {
        minimax: state.store.has(Credential::MiniMax),
    }
}

/// Store a key. Returns an optional advisory warning — a suspicious key is
/// still saved, because only the API can say whether it works (SPEC §2).
#[tauri::command]
fn save_key(state: tauri::State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    let warning = suspicious(&key).map(str::to_owned);
    state
        .store
        .set(Credential::MiniMax, &key)
        .map_err(|e| e.to_string())?;
    Ok(warning)
}

#[tauri::command]
fn clear_key(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .store
        .delete(Credential::MiniMax)
        .map_err(|e| e.to_string())
}

/// Cancel the generation in flight. SPEC §3.3: this drops the request rather
/// than polling, and nothing is written, so there is no partial file.
#[tauri::command]
fn cancel(state: tauri::State<'_, AppState>) {
    if let Some(token) = state.current.lock().unwrap().take() {
        token.cancel();
    }
}

#[tauri::command]
async fn generate(
    state: tauri::State<'_, AppState>,
    req: ComposeRequest,
) -> Result<TakeResult, Failure> {
    let key = state
        .store
        .get(Credential::MiniMax)
        .map_err(|e| plain(e.to_string()))?
        .ok_or_else(|| plain("No API key saved. Add one in Settings.".to_owned()))?;

    let audio_setting = AudioSetting::new(req.format, req.sample_rate, req.bitrate);
    let mut request = GenerationRequest::new(req.model, OutputFormat::Url)
        .prompt(req.caption.trim())
        .instrumental(req.instrumental)
        .audio_setting(audio_setting);

    // The lyrics field carries structure even for instrumentals — it is what
    // controls take length (SPEC §3.5), so it is never dropped.
    if !req.lyrics.trim().is_empty() {
        request = request.lyrics(req.lyrics.trim_end());
    }

    let token = CancellationToken::new();
    *state.current.lock().unwrap() = Some(token.clone());

    let started = std::time::Instant::now();
    let outcome = generate_and_fetch(&key, &request, &token).await;
    let elapsed = started.elapsed().as_secs_f64();

    state.current.lock().unwrap().take();

    let (take, bytes) = outcome?;

    // SPEC §5: one folder per song, takes inside, receipt beside the audio.
    let stored = state
        .library()
        .save(NewTake {
            title: &req.title,
            model: req.model.as_str(),
            caption: req.caption.trim(),
            lyrics: req.lyrics.trim_end(),
            instrumental: req.instrumental,
            lyrics_optimizer: false,
            request: serde_json::to_value(&request).unwrap_or(serde_json::Value::Null),
            extra_info: serde_json::to_value(&take.extra).unwrap_or(serde_json::Value::Null),
            trace_id: take.trace_id.clone(),
            call_secs: elapsed,
            duration_secs: take.extra.duration_secs(),
            audio: &bytes,
            extension: req.format.extension(),
            sample_rate: req.sample_rate.value(),
            bitrate: req.bitrate.map(Bitrate::value),
        })
        .map_err(|e| plain(e.to_string()))?;

    Ok(TakeResult {
        take: stored,
        trace_id: take.trace_id,
        bytes: bytes.len() as u64,
    })
}

// -------------------------------------------------------------------- library

#[tauri::command]
fn list_takes(state: tauri::State<'_, AppState>) -> Result<Vec<StoredTake>, String> {
    state.library().scan().map_err(|e| e.to_string())
}

/// SPEC §5.3. `stars` is 0–5; 0 clears the rating.
#[tauri::command]
fn set_rating(state: tauri::State<'_, AppState>, dir: String, stars: u8) -> Result<(), String> {
    state
        .library()
        .set_rating(&dir, stars)
        .map_err(|e| e.to_string())
}

/// SPEC §5.4: only ever removes the app's own directory. The path is checked
/// against the library root in Rust, not trusted from the webview.
#[tauri::command]
fn delete_take(state: tauri::State<'_, AppState>, dir: String) -> Result<(), String> {
    state.library().delete_take(&dir).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- lyrics

/// The canonical structure tags (SPEC §3.1). The UI builds its tag bar from
/// this rather than keeping a second copy that could drift.
#[tauri::command]
fn structure_tags() -> Vec<&'static str> {
    studio::TAGS.to_vec()
}

/// Bracketed text the API would sing rather than read as structure.
///
/// This is cheap and local, so the UI can call it on every keystroke. It is a
/// warning, never a block: a writer may genuinely want a bracketed line sung.
#[tauri::command]
fn lint_lyrics(lyrics: String) -> Vec<studio::StrayTag> {
    studio::stray_tags(&lyrics)
}

#[tauri::command]
fn library_root(state: tauri::State<'_, AppState>) -> String {
    state.library().root().to_string_lossy().into_owned()
}

/// Read a `song.md` so the form can be restored from it. SPEC §7.3.
///
/// The path comes from a native drag-and-drop event, so it is a real path the
/// user chose rather than anything the webview invented — but it is still read
/// here in Rust, never by the frontend.
#[tauri::command]
fn read_recipe(path: String) -> Result<Recipe, String> {
    let p = std::path::Path::new(&path);

    match p.extension().and_then(|e| e.to_str()) {
        Some("md" | "markdown" | "txt") => {}
        _ => {
            return Err(format!(
                "{} is not a recipe — drop a song.md file",
                p.file_name().unwrap_or_default().to_string_lossy()
            ))
        }
    }

    Recipe::read(p).map_err(|e| e.to_string())
}

/// A take's own settings, read from its immutable run.json — used when the
/// history rail loads a take back into the compose form. SPEC §7.3.
///
/// Deliberately not `song.md`: that file is rewritten on every generate and
/// reflects only the newest take, so it would load the wrong take's caption
/// and lyrics for anything but the most recent one.
#[tauri::command]
fn take_recipe(state: tauri::State<'_, AppState>, dir: String) -> Result<Recipe, String> {
    state
        .library()
        .recipe_from_take(&dir)
        .map_err(|e| e.to_string())
}

// -------------------------------------------------------------------- settings

/// SPEC §3.1's six model IDs, minus the two cover models — Compose never sets
/// reference audio, so a cover model here would fail validation on every
/// single request. Cover models come back once the Cover tab exists.
#[tauri::command]
fn available_models() -> Vec<ModelInfo> {
    Model::ALL
        .into_iter()
        .filter(|m| !m.is_cover())
        .map(|m| ModelInfo {
            id: m.as_str().to_owned(),
            rpm: m.rpm(),
            free_tier: m.is_free_tier(),
        })
        .collect()
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> SettingsPayload {
    settings_payload(&state.settings.lock().unwrap())
}

/// Persists model/format/sample-rate/bitrate and an optional library root
/// override, normalising the bitrate-vs-format rule (SPEC §3.1: mp3 only)
/// rather than trusting whatever combination the UI last had on screen.
///
/// A custom library root is probed with `create_dir_all` before it is
/// accepted — better to fail here with a clear message than to accept a path
/// that turns out to be unwritable the next time a take tries to save there.
#[tauri::command]
fn save_settings(
    state: tauri::State<'_, AppState>,
    mut settings: Settings,
) -> Result<SettingsPayload, String> {
    settings.normalise();

    if let Some(root) = &settings.library_root {
        std::fs::create_dir_all(root)
            .map_err(|e| format!("Can't use {} as the library folder: {e}", root.display()))?;
    }

    settings.save().map_err(|e| e.to_string())?;
    let payload = settings_payload(&settings);
    *state.settings.lock().unwrap() = settings;
    Ok(payload)
}

/// A native folder picker, so the library location can be changed without
/// typing a path by hand. `None` means the user cancelled — not an error.
///
/// Goes straight to `rfd` rather than Tauri's dialog plugin: a native picker
/// needs no filesystem capability of its own (it is the OS choosing the path,
/// not the webview), so this avoids a plugin dependency and a
/// `capabilities/default.json` entry for what is otherwise a two-line call.
#[tauri::command]
fn pick_library_folder() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a library folder")
        .pick_folder()
        .map(|p: PathBuf| p.to_string_lossy().into_owned())
}

/// Generate, then immediately turn the expiring URL into bytes (SPEC §6.4).
async fn generate_and_fetch(
    key: &str,
    request: &GenerationRequest,
    token: &CancellationToken,
) -> Result<(minimax::Generation, Vec<u8>), Failure> {
    let client = Client::new(key).map_err(Failure::from)?;
    let take = client
        .generate(request, token)
        .await
        .map_err(Failure::from)?;
    let bytes = client
        .fetch_audio(&take.audio, token)
        .await
        .map_err(Failure::from)?;
    Ok((take, bytes))
}

// ------------------------------------------------------------------- helpers

fn plain(message: String) -> Failure {
    Failure {
        message,
        code: None,
        trace_id: None,
        cancelled: false,
        fields: Vec::new(),
    }
}

/// Show a take in the file manager. macOS only for now.
#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("Reveal is macOS-only for now".to_owned())
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            key_status,
            save_key,
            clear_key,
            generate,
            cancel,
            reveal,
            list_takes,
            set_rating,
            delete_take,
            library_root,
            read_recipe,
            take_recipe,
            structure_tags,
            lint_lyrics,
            available_models,
            get_settings,
            save_settings,
            pick_library_folder
        ])
        .run(tauri::generate_context!())
        .expect("could not start MusicMaxxer");
}
