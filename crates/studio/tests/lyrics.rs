//! Catching bracketed text that would be sung. SPEC §3.1, §3.5.

use studio::lyrics::{is_clean, stray_tags, TAGS};

#[test]
fn the_documented_tags_are_all_recognised() {
    // SPEC §3.1 lists exactly these fourteen.
    assert_eq!(TAGS.len(), 14);
    let sheet = TAGS.join("\n");
    assert!(is_clean(&sheet), "{:?}", stray_tags(&sheet));
}

#[test]
fn ordinary_lyrics_are_clean() {
    let sheet = "[Verse]\nSix flights up and the elevator's out\n\n[Chorus]\nTill the morning gets it right";
    assert!(is_clean(sheet));
}

/// The actual failure, observed 2026-08-17: this was sung aloud.
#[test]
fn a_performance_note_inside_a_tag_is_caught() {
    let sheet =
        "[Verse 1 - hushed breathy male tenor, close-miked, half-spoken, flat and drained]\n\
                 Six flights up";

    let strays = stray_tags(sheet);
    assert_eq!(strays.len(), 1);
    assert_eq!(strays[0].line, 1);
    assert!(strays[0].text.contains("hushed breathy"));
    assert_eq!(strays[0].suggestion.as_deref(), Some("[Verse]"));
}

/// The subtle one: a numbered section is not a tag either.
#[test]
fn a_numbered_section_is_not_a_tag() {
    let strays = stray_tags("[Verse 1]\nSix flights up\n\n[Verse 2]\nCold coffee");

    assert_eq!(strays.len(), 2);
    assert_eq!(strays[0].suggestion.as_deref(), Some("[Verse]"));
    assert_eq!(strays[1].line, 4);
}

/// A two-word tag must not be shadowed by its one-word ending.
#[test]
fn post_chorus_suggests_post_chorus_not_chorus() {
    let strays = stray_tags("[Post Chorus 2]\nwords");
    assert_eq!(strays[0].suggestion.as_deref(), Some("[Post Chorus]"));

    let build = stray_tags("[Build Up - slowly]\nwords");
    assert_eq!(build[0].suggestion.as_deref(), Some("[Build Up]"));
}

#[test]
fn something_unrelated_gets_no_suggestion() {
    let strays = stray_tags("[Whispered]\nwords");
    assert_eq!(strays.len(), 1);
    assert_eq!(strays[0].suggestion, None);
}

#[test]
fn tags_are_case_and_space_insensitive() {
    // The model accepts these; the writer should not be nagged about them.
    assert!(is_clean("[verse]\nwords"));
    assert!(is_clean("[PRE CHORUS]\nwords"));
    assert!(is_clean("[Pre  Chorus]\nwords"));
    assert!(is_clean("  [Chorus]  \nwords"));
}

#[test]
fn a_stray_mid_line_is_caught_too() {
    // Brackets do not have to be alone on a line to be sung.
    let strays = stray_tags("She said [softly] and left");
    assert_eq!(strays.len(), 1);
    assert_eq!(strays[0].text, "[softly]");
}

#[test]
fn several_strays_on_one_line_are_all_reported() {
    let strays = stray_tags("[Verse 1] and [ad lib]");
    assert_eq!(strays.len(), 2);
    assert_eq!(strays[1].text, "[ad lib]");
}

#[test]
fn an_unclosed_bracket_is_ordinary_text() {
    // A lyric that happens to contain a bracket is not a broken tag.
    assert!(is_clean("The sign said [ and nothing else"));
}

#[test]
fn line_numbers_are_one_based_and_match_the_editor() {
    let sheet = "[Verse]\nfirst line\n\n[Chorus 2]\nsecond";
    let strays = stray_tags(sheet);
    assert_eq!(strays.len(), 1);
    assert_eq!(strays[0].line, 4, "the writer counts from 1");
}

#[test]
fn empty_lyrics_are_clean() {
    assert!(is_clean(""));
    assert!(is_clean("\n\n"));
}
