//! Library layout on disk. SPEC §5, §11.

use serde_json::json;
use studio::library::{slugify, TakeMeta};
use studio::{Library, LibraryError, NewTake};
use tempfile::TempDir;

fn lib() -> (TempDir, Library) {
    let dir = TempDir::new().unwrap();
    let lib = Library::new(dir.path());
    (dir, lib)
}

fn take<'a>(title: &'a str, caption: &'a str, audio: &'a [u8]) -> NewTake<'a> {
    NewTake {
        title,
        model: "music-3.0-free",
        caption,
        lyrics: "[Verse]\nSix flights up",
        instrumental: false,
        lyrics_optimizer: false,
        request: json!({ "model": "music-3.0-free", "prompt": caption }),
        extra_info: json!({ "music_duration": 106405 }),
        trace_id: Some("trace-abc".to_owned()),
        call_secs: 86.7,
        duration_secs: 106.4,
        audio,
        extension: "wav",
        sample_rate: 44100,
        bitrate: None,
    }
}

// --------------------------------------------------------------- default root

/// Regression test: `default_root()` used to read `HOME` directly, which
/// Windows does not reliably set outside a shell — launched from the Start
/// Menu, it silently fell back to `.`, the app's own working directory.
#[test]
fn default_root_resolves_a_real_per_os_directory_not_the_working_directory() {
    let root = Library::default_root();
    assert_ne!(
        root,
        std::path::PathBuf::from("."),
        "must resolve a real directory, not silently fall back to cwd: {}",
        root.display()
    );

    #[cfg(target_os = "windows")]
    assert!(
        root.ends_with("MusicMaxxer"),
        "expected .../MusicMaxxer under LocalAppData, got {}",
        root.display()
    );

    #[cfg(not(target_os = "windows"))]
    assert!(
        root.ends_with("MiniMaxMusic"),
        "expected ~/MiniMaxMusic, got {}",
        root.display()
    );
}

// ------------------------------------------------------------------ slugging

#[test]
fn slugs_follow_the_spec() {
    assert_eq!(slugify("Six Flights Up"), "six-flights-up");
    assert_eq!(
        slugify("  Hold On (gospel rework)  "),
        "hold-on-gospel-rework"
    );
    assert_eq!(slugify("Third Coffee!!!"), "third-coffee");
    assert_eq!(slugify("///"), "untitled-song");
    assert_eq!(slugify(""), "untitled-song");
    // Capped at 40 characters, and never left ending on a hyphen.
    assert!(slugify(&"a b ".repeat(40)).len() <= 40);
    assert!(!slugify(&"a b ".repeat(40)).ends_with('-'));
}

// ------------------------------------------------------------ the §5 layout

#[test]
fn a_take_lands_in_song_slash_take() {
    let (dir, lib) = lib();
    let stored = lib
        .save(take("Six Flights Up", "Acid jazz", b"RIFFDATA"))
        .unwrap();

    let expected = dir.path().join("six-flights-up").join("take-01");
    assert_eq!(stored.dir, expected.to_string_lossy());
    assert_eq!(stored.take, 1);
    assert!(expected.join("track.wav").exists());
    assert!(expected.join("run.json").exists());
    // Nothing is written until the take is rated (SPEC §5.3).
    assert!(!expected.join("meta.json").exists());
}

#[test]
fn takes_of_one_song_share_a_folder_and_count_up() {
    let (dir, lib) = lib();
    for _ in 0..3 {
        lib.save(take("Six Flights Up", "Acid jazz", b"AUDIO"))
            .unwrap();
    }

    let song = dir.path().join("six-flights-up");
    for n in ["take-01", "take-02", "take-03"] {
        assert!(song.join(n).exists(), "{n} missing");
    }
    // One folder for the song, not three.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

/// SPEC §11: deleting a take must not cause the next one to reuse its number.
#[test]
fn take_numbers_are_never_reused() {
    let (dir, lib) = lib();
    lib.save(take("Six Flights Up", "a", b"A")).unwrap();
    let second = lib.save(take("Six Flights Up", "b", b"B")).unwrap();

    lib.delete_take(&second.dir).unwrap();
    let third = lib.save(take("Six Flights Up", "c", b"C")).unwrap();

    assert_eq!(third.take, 3, "take-02 was deleted; 02 must not come back");
    assert!(dir.path().join("six-flights-up/take-03").exists());
}

/// SPEC §5.1: two *different* titles that slugify alike get a suffix.
#[test]
fn colliding_slugs_get_a_suffix() {
    let (dir, lib) = lib();
    lib.save(take("Third Coffee", "a", b"A")).unwrap();
    let other = lib.save(take("third coffee!", "b", b"B")).unwrap();

    assert_eq!(other.song_slug, "third-coffee-2");
    assert!(dir.path().join("third-coffee").exists());
    assert!(dir.path().join("third-coffee-2").exists());
}

/// ...but the *same* title keeps its folder, however it is typed.
#[test]
fn the_same_title_reuses_its_folder() {
    let (_dir, lib) = lib();
    let first = lib.save(take("Six Flights Up", "a", b"A")).unwrap();
    let second = lib.save(take("  Six Flights Up  ", "b", b"B")).unwrap();

    assert_eq!(first.song_slug, second.song_slug);
    assert_eq!(second.take, 2);
}

#[test]
fn an_empty_title_becomes_untitled_song() {
    let (_dir, lib) = lib();
    let stored = lib.save(take("   ", "a", b"A")).unwrap();

    assert_eq!(stored.song_slug, "untitled-song");
    assert_eq!(stored.title, "Untitled song");
}

// ------------------------------------------------------------------ receipts

#[test]
fn the_receipt_carries_everything_needed_to_reproduce() {
    let (_dir, lib) = lib();
    let stored = lib
        .save(take("Six Flights Up", "Acid jazz, 104 BPM", b"AUDIO"))
        .unwrap();

    let raw = std::fs::read_to_string(format!("{}/run.json", stored.dir)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(v["title"], "Six Flights Up");
    assert_eq!(v["model"], "music-3.0-free");
    assert_eq!(v["trace_id"], "trace-abc");
    assert_eq!(v["request"]["prompt"], "Acid jazz, 104 BPM");
    assert_eq!(v["bytes"], 5);
    assert!(v["call_secs"].as_f64().unwrap() > 0.0);
    assert!(v["created_at"].as_str().unwrap().contains('T'), "ISO-8601");
    // SPEC §6.4: the AI disclosure travels with the take.
    assert!(v["generated_by_ai"].as_str().unwrap().contains("MiniMax"));
}

/// SPEC §7: the recipe sits at the song root, beside the take folders, and is
/// rewritten on every generate so it reflects the newest inputs.
#[test]
fn the_recipe_lands_at_the_song_root_and_is_rewritten() {
    let (dir, lib) = lib();
    lib.save(take("Six Flights Up", "first caption", b"A"))
        .unwrap();

    let song_md = dir.path().join("six-flights-up/song.md");
    assert!(
        song_md.exists(),
        "song.md should sit beside the take folders"
    );
    assert!(std::fs::read_to_string(&song_md)
        .unwrap()
        .contains("first caption"));

    lib.save(take("Six Flights Up", "second caption", b"B"))
        .unwrap();
    let text = std::fs::read_to_string(&song_md).unwrap();
    assert!(
        text.contains("second caption"),
        "rewritten for the newest take"
    );
    assert!(!text.contains("first caption"));
}

/// A recipe written by the library must parse back through the same reader the
/// drag-and-drop path uses.
#[test]
fn a_written_recipe_reads_back() {
    let (dir, lib) = lib();
    lib.save(take("Six Flights Up", "Acid jazz, 104 BPM", b"A"))
        .unwrap();

    let recipe = studio::Recipe::read(&dir.path().join("six-flights-up/song.md")).unwrap();

    assert_eq!(recipe.title, "Six Flights Up");
    assert_eq!(recipe.caption, "Acid jazz, 104 BPM");
    assert!(recipe.lyrics.contains("Six flights up"));
    assert_eq!(recipe.audio.format, "wav");
    assert_eq!(recipe.audio.sample_rate, 44100);
}

/// SPEC §7.3: clicking a take in the history rail must load THAT take's own
/// settings. `song.md` is rewritten on every generate and reflects only the
/// newest take (see `the_recipe_lands_at_the_song_root_and_is_rewritten`
/// above), so reading it back for an older take would silently hand back a
/// different take's caption and lyrics. Regression test for exactly that bug.
#[test]
fn recipe_from_take_reflects_that_takes_own_receipt_not_the_shared_song_md() {
    let (_dir, lib) = lib();
    let first = lib
        .save(take("Six Flights Up", "first caption", b"A"))
        .unwrap();
    // A second take overwrites song.md with different content.
    lib.save(take("Six Flights Up", "second caption", b"B"))
        .unwrap();

    let recipe = lib.recipe_from_take(&first.dir).unwrap();
    assert_eq!(
        recipe.caption, "first caption",
        "must read take-01's own receipt, not the song's rewritten song.md"
    );
}

/// Fields the compose form does not yet expose (audio settings,
/// lyrics_optimizer) live inside run.json's `request` blob, not as top-level
/// receipt fields — this is a best-effort read with sane fallbacks.
#[test]
fn recipe_from_take_falls_back_when_the_request_lacks_audio_settings() {
    let (_dir, lib) = lib();
    // The test fixture's request has no audio_setting or lyrics_optimizer key.
    let stored = lib.save(take("Six Flights Up", "a", b"A")).unwrap();

    let recipe = lib.recipe_from_take(&stored.dir).unwrap();
    assert_eq!(recipe.audio.format, "wav");
    assert_eq!(recipe.audio.sample_rate, 44100);
    assert_eq!(recipe.audio.bitrate, None);
    assert!(!recipe.lyrics_optimizer);
    assert!(recipe.lyrics.contains("Six flights up"));
}

/// The same containment rule as deletion: a path outside the library must
/// never be handed back for reading.
#[test]
fn recipe_from_take_refuses_a_path_outside_the_root() {
    let (_dir, lib) = lib();
    let outside = TempDir::new().unwrap();

    let err = lib
        .recipe_from_take(&outside.path().to_string_lossy())
        .unwrap_err();
    assert!(matches!(err, LibraryError::OutsideRoot(_)));
}

// ------------------------------------------------------------------- ratings

#[test]
fn rating_writes_meta_and_scan_reads_it_back() {
    let (_dir, lib) = lib();
    let stored = lib.save(take("Six Flights Up", "a", b"A")).unwrap();

    assert_eq!(lib.scan().unwrap()[0].rating, 0);

    lib.set_rating(&stored.dir, 4).unwrap();
    assert_eq!(lib.scan().unwrap()[0].rating, 4);

    let raw = std::fs::read_to_string(format!("{}/meta.json", stored.dir)).unwrap();
    let meta: TakeMeta = serde_json::from_str(&raw).unwrap();
    assert_eq!(meta.rating, 4);
    assert!(meta.rated_at.is_some());
}

#[test]
fn rating_zero_clears_it() {
    let (_dir, lib) = lib();
    let stored = lib.save(take("Six Flights Up", "a", b"A")).unwrap();

    lib.set_rating(&stored.dir, 5).unwrap();
    lib.set_rating(&stored.dir, 0).unwrap();

    assert_eq!(lib.scan().unwrap()[0].rating, 0);
    let raw = std::fs::read_to_string(format!("{}/meta.json", stored.dir)).unwrap();
    let meta: TakeMeta = serde_json::from_str(&raw).unwrap();
    assert!(meta.rated_at.is_none(), "an unrated take has no rated_at");
}

#[test]
fn ratings_are_clamped_to_five() {
    let (_dir, lib) = lib();
    let stored = lib.save(take("Six Flights Up", "a", b"A")).unwrap();

    lib.set_rating(&stored.dir, 99).unwrap();
    assert_eq!(lib.scan().unwrap()[0].rating, 5);
}

/// Rating must not touch the receipt — that is the whole reason it is a
/// separate file (SPEC §5.3).
#[test]
fn rating_leaves_the_receipt_untouched() {
    let (_dir, lib) = lib();
    let stored = lib.save(take("Six Flights Up", "a", b"A")).unwrap();
    let run = format!("{}/run.json", stored.dir);

    let before = std::fs::read_to_string(&run).unwrap();
    lib.set_rating(&stored.dir, 3).unwrap();
    assert_eq!(std::fs::read_to_string(&run).unwrap(), before);
}

// -------------------------------------------------------------------- safety

/// SPEC §5.4: "never touch files outside the library root."
#[test]
fn deleting_outside_the_root_is_refused() {
    let (_dir, lib) = lib();
    let outside = TempDir::new().unwrap();
    let victim = outside.path().join("precious");
    std::fs::create_dir_all(&victim).unwrap();

    let err = lib.delete_take(victim.to_str().unwrap()).unwrap_err();
    assert!(matches!(err, LibraryError::OutsideRoot(_)), "{err:?}");
    assert!(victim.exists(), "a refused delete must not delete anything");
}

#[test]
fn traversal_out_of_the_root_is_refused() {
    let (dir, lib) = lib();
    lib.save(take("Six Flights Up", "a", b"A")).unwrap();

    let escape = dir.path().join("six-flights-up/../../..");
    assert!(matches!(
        lib.delete_take(escape.to_str().unwrap()),
        Err(LibraryError::OutsideRoot(_))
    ));
}

#[test]
fn the_root_itself_cannot_be_deleted() {
    let (dir, lib) = lib();
    assert!(lib.delete_take(dir.path().to_str().unwrap()).is_err());
    assert!(dir.path().exists());
}

#[test]
fn deleting_the_last_take_removes_the_empty_song_folder() {
    let (dir, lib) = lib();
    let stored = lib.save(take("Six Flights Up", "a", b"A")).unwrap();

    lib.delete_take(&stored.dir).unwrap();
    assert!(!dir.path().join("six-flights-up").exists());
}

// -------------------------------------------------------------------- scanning

#[test]
fn scanning_an_empty_or_missing_library_is_not_an_error() {
    let (_dir, lib) = lib();
    assert!(lib.scan().unwrap().is_empty());

    let missing = Library::new("/nonexistent/path/for/a/test");
    assert!(missing.scan().unwrap().is_empty());
}

#[test]
fn stray_folders_do_not_break_the_history_list() {
    let (dir, lib) = lib();
    lib.save(take("Six Flights Up", "a", b"A")).unwrap();

    // Something a user dropped in, and a take folder with no receipt.
    std::fs::create_dir_all(dir.path().join("notes")).unwrap();
    std::fs::create_dir_all(dir.path().join("six-flights-up/take-99")).unwrap();
    std::fs::write(dir.path().join("stray.txt"), "hello").unwrap();

    let takes = lib.scan().unwrap();
    assert_eq!(takes.len(), 1, "only the real take should be listed");
}

#[test]
fn scan_returns_newest_first() {
    let (_dir, lib) = lib();
    lib.save(take("Alpha", "a", b"A")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    lib.save(take("Beta", "b", b"B")).unwrap();

    let takes = lib.scan().unwrap();
    assert_eq!(takes[0].title, "Beta", "newest first");
    assert_eq!(takes[1].title, "Alpha");
}
