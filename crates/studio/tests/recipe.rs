//! `song.md` round trips. SPEC §7, §11.

use studio::recipe::{AudioSettings, CoverRef, Recipe, RecipeError, UNRECORDED_LYRICS};

fn sample() -> Recipe {
    Recipe {
        title: "Six Flights Up".to_owned(),
        created: "2026-08-17T14:22:05Z".to_owned(),
        model: "music-3.0-free".to_owned(),
        instrumental: false,
        lyrics_optimizer: false,
        audio: AudioSettings {
            format: "wav".to_owned(),
            sample_rate: 44100,
            bitrate: None,
        },
        cover: None,
        caption: "Acid jazz, 104 BPM, E minor. Live kit, slap bass, Rhodes.".to_owned(),
        lyrics: "[Verse]\nSix flights up and the elevator's out\n\n[Chorus]\nTill the morning gets it right"
            .to_owned(),
    }
}

/// SPEC §11: "generate, hand-edit the resulting song.md, drop it back, and
/// verify every control returns to the edited state."
#[test]
fn a_recipe_survives_a_round_trip() {
    let original = sample();
    let parsed = Recipe::from_markdown(&original.to_markdown()).unwrap();
    assert_eq!(parsed, original);
}

#[test]
fn the_written_file_looks_like_the_spec() {
    let md = sample().to_markdown();

    assert!(md.starts_with("---\nminimax_music: 1\n"));
    assert!(md.contains("title: Six Flights Up\n"));
    assert!(md.contains("audio:\n  format: wav\n  sample_rate: 44100\n"));
    assert!(md.contains("\n## Caption\n\n"));
    assert!(md.contains("\n## Lyrics\n\n"));
    // SPEC §7.1: bitrate appears only for mp3.
    assert!(!md.contains("bitrate"));
}

#[test]
fn bitrate_is_written_only_for_mp3() {
    let mut r = sample();
    r.audio = AudioSettings {
        format: "mp3".to_owned(),
        sample_rate: 44100,
        bitrate: Some(256000),
    };
    assert!(r.to_markdown().contains("  bitrate: 256000\n"));

    // Same bitrate on a wav recipe is not written, and does not come back.
    r.audio.format = "wav".to_owned();
    let md = r.to_markdown();
    assert!(!md.contains("bitrate"));
    assert_eq!(Recipe::from_markdown(&md).unwrap().audio.bitrate, None);
}

/// The regression from the mockup: `reference_sha256` has digits in the key,
/// and a key pattern that excluded digits dropped the whole cover block.
#[test]
fn a_cover_block_round_trips_including_the_hash() {
    let mut r = sample();
    r.cover = Some(CoverRef {
        reference_file: "demo-take.wav".to_owned(),
        reference_sha256: "9f2c4b71ae03d8c5f6b2".to_owned(),
        rights_confirmed: true,
    });

    let back = Recipe::from_markdown(&r.to_markdown()).unwrap();
    let cover = back.cover.expect("cover block survived");
    assert_eq!(cover.reference_sha256, "9f2c4b71ae03d8c5f6b2");
    assert_eq!(cover.reference_file, "demo-take.wav");
    assert!(cover.rights_confirmed);
}

#[test]
fn a_title_with_a_colon_survives() {
    let mut r = sample();
    r.title = "Hold On: the gospel rework".to_owned();

    let md = r.to_markdown();
    assert!(md.contains("title: \"Hold On: the gospel rework\""));
    assert_eq!(Recipe::from_markdown(&md).unwrap().title, r.title);
}

// -------------------------------------------------------------- placeholders

/// SPEC §7.2: a lone parenthesised line is a placeholder, not content.
#[test]
fn placeholders_read_back_as_empty() {
    let mut r = sample();
    r.lyrics = String::new();
    r.lyrics_optimizer = true;

    let md = r.to_markdown();
    assert!(md.contains(UNRECORDED_LYRICS));

    let back = Recipe::from_markdown(&md).unwrap();
    assert_eq!(back.lyrics, "", "the placeholder must not become lyrics");
    assert!(back.lyrics_optimizer);
}

#[test]
fn a_real_parenthetical_lyric_is_not_mistaken_for_a_placeholder() {
    // Multi-line content that happens to open with a bracket is content.
    let mut r = sample();
    r.lyrics = "(spoken)\nSix flights up".to_owned();

    assert_eq!(
        Recipe::from_markdown(&r.to_markdown()).unwrap().lyrics,
        r.lyrics
    );
}

// ------------------------------------------------------------ hand-editing

/// The point of the format: someone edits it in a text editor and drops it back.
#[test]
fn a_hand_edited_file_parses() {
    let edited = "---\n\
        minimax_music: 1\n\
        title: Neon Off-Ramp\n\
        model: music-3.0-free\n\
        instrumental: false\n\
        lyrics_optimizer: false\n\
        audio:\n\
        \x20 format: wav\n\
        \x20 sample_rate: 44100\n\
        ---\n\n\
        ## Caption\n\n\
        Synthwave, 118 BPM, F# minor.\n\n\
        ## Lyrics\n\n\
        [Verse]\n\
        Half past the exit and the dashboard glows\n";

    let r = Recipe::from_markdown(edited).unwrap();
    assert_eq!(r.title, "Neon Off-Ramp");
    assert_eq!(r.caption, "Synthwave, 118 BPM, F# minor.");
    assert!(r.lyrics.starts_with("[Verse]"));
    assert!(!r.instrumental);
}

/// A mangled front matter must still recover the writing — that is the whole
/// reason the sections are found by heading rather than by position.
#[test]
fn a_broken_front_matter_still_recovers_the_writing() {
    let broken = "## Caption\n\nAcid jazz, 104 BPM.\n\n## Lyrics\n\n[Verse]\nstill here\n";

    let r = Recipe::from_markdown(broken).unwrap();
    assert_eq!(r.caption, "Acid jazz, 104 BPM.");
    assert!(r.lyrics.contains("still here"));
    assert_eq!(r.title, "Untitled song");
}

#[test]
fn a_horizontal_rule_in_the_body_does_not_end_the_front_matter() {
    let md = "---\ntitle: Rules\nmodel: music-3.0-free\n---\n\n## Caption\n\nBefore\n\n---\n\n## Lyrics\n\n[Verse]\nAfter\n";

    let r = Recipe::from_markdown(md).unwrap();
    assert_eq!(r.title, "Rules");
    assert!(r.lyrics.contains("After"));
}

#[test]
fn windows_line_endings_parse() {
    let md = sample().to_markdown().replace('\n', "\r\n");
    let r = Recipe::from_markdown(&md).unwrap();
    assert_eq!(r.title, "Six Flights Up");
    assert!(r.caption.starts_with("Acid jazz"));
}

/// SPEC §7.1: refuse a version we do not understand rather than guessing.
#[test]
fn a_newer_format_version_is_refused() {
    let md = "---\nminimax_music: 99\ntitle: From the future\n---\n\n## Caption\n\nx\n";

    assert!(matches!(
        Recipe::from_markdown(md),
        Err(RecipeError::Version { found: 99, ours: 1 })
    ));
}

#[test]
fn an_instrumental_recipe_omits_an_empty_lyrics_section() {
    let mut r = sample();
    r.instrumental = true;
    r.lyrics = String::new();

    let md = r.to_markdown();
    assert!(!md.contains("## Lyrics"));
    assert!(Recipe::from_markdown(&md).unwrap().instrumental);
}

/// An instrumental still carries structure, because that is what sets the
/// take's length (SPEC §3.5).
#[test]
fn an_instrumental_keeps_its_structure_tags() {
    let mut r = sample();
    r.instrumental = true;
    r.lyrics = "[Intro]\n\n[Build Up]\n\n[Inst]\n\n[Outro]".to_owned();

    let back = Recipe::from_markdown(&r.to_markdown()).unwrap();
    assert_eq!(back.lyrics, r.lyrics);
}
