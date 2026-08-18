//! Persisted output settings. SPEC §6.3.

use minimax::{AudioFormat, Bitrate, Model, SampleRate};
use studio::Settings;
use tempfile::TempDir;

fn path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("settings.json")
}

#[test]
fn defaults_match_masters() {
    // SPEC §6.3: default to wav @ 44100 -- masters, not previews.
    let s = Settings::default();
    assert_eq!(s.model, Model::Music30Free);
    assert_eq!(s.format, AudioFormat::Wav);
    assert_eq!(s.sample_rate, SampleRate::Hz44100);
    assert_eq!(s.bitrate, None);
    assert_eq!(s.library_root, None);
}

#[test]
fn a_saved_settings_file_reads_back_identical() {
    let dir = TempDir::new().unwrap();
    let s = Settings {
        model: Model::Music30,
        format: AudioFormat::Mp3,
        sample_rate: SampleRate::Hz32000,
        bitrate: Some(Bitrate::Kbps128),
        library_root: Some("/tmp/somewhere".into()),
    };

    s.save_to(&path(&dir)).unwrap();
    assert_eq!(Settings::load_from(&path(&dir)), s);
}

#[test]
fn a_missing_file_loads_as_defaults_not_an_error() {
    let dir = TempDir::new().unwrap();
    assert_eq!(Settings::load_from(&path(&dir)), Settings::default());
}

#[test]
fn a_corrupt_file_loads_as_defaults_not_a_crash() {
    let dir = TempDir::new().unwrap();
    std::fs::write(path(&dir), "{ not json at all").unwrap();
    assert_eq!(Settings::load_from(&path(&dir)), Settings::default());
}

/// The regression this guards: a settings.json hand-edited (or corrupted) to
/// carry an undocumented sample rate must not silently take effect — it
/// should be treated exactly like a missing file, not partially trusted.
#[test]
fn an_undocumented_value_is_rejected_wholesale_not_partially_trusted() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        path(&dir),
        r#"{"model":"music-3.0-free","format":"wav","sample_rate":99999}"#,
    )
    .unwrap();
    assert_eq!(Settings::load_from(&path(&dir)), Settings::default());
}

#[test]
fn effective_library_root_falls_back_to_the_platform_default() {
    let s = Settings::default();
    assert_eq!(s.effective_library_root(), studio::Library::default_root());
}

#[test]
fn effective_library_root_prefers_an_explicit_override() {
    let s = Settings {
        library_root: Some("/custom/path".into()),
        ..Settings::default()
    };
    assert_eq!(
        s.effective_library_root(),
        std::path::PathBuf::from("/custom/path")
    );
}

/// SPEC §3.1: bitrate is mp3-only. Reuses AudioSetting::new's rule rather
/// than restating it -- this test is really pinning that reuse.
#[test]
fn normalise_drops_bitrate_for_a_non_mp3_format() {
    let mut s = Settings {
        format: AudioFormat::Wav,
        bitrate: Some(Bitrate::Kbps256),
        ..Settings::default()
    };
    s.normalise();
    assert_eq!(s.bitrate, None);
}

#[test]
fn normalise_keeps_bitrate_for_mp3() {
    let mut s = Settings {
        format: AudioFormat::Mp3,
        bitrate: Some(Bitrate::Kbps256),
        ..Settings::default()
    };
    s.normalise();
    assert_eq!(s.bitrate, Some(Bitrate::Kbps256));
}
