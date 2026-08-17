//! Write a recipe, hand-edit it the way a person would, read it back.
//! SPEC §11's recipe check, runnable without the GUI.

use std::path::PathBuf;

fn main() {
    let dir = std::env::temp_dir().join("minimax-recipe-check");
    std::fs::create_dir_all(&dir).unwrap();

    let original = studio::Recipe {
        title: "Hold On: the gospel rework".to_owned(),
        created: "2026-08-17T14:22:05Z".to_owned(),
        model: "music-3.0-free".to_owned(),
        instrumental: false,
        lyrics_optimizer: false,
        audio: studio::recipe::AudioSettings {
            format: "wav".to_owned(),
            sample_rate: 44100,
            bitrate: None,
        },
        cover: None,
        caption: "Slow gospel-soul rework, 68 BPM, Hammond organ.".to_owned(),
        lyrics: "[Verse]\nWe drove the long way\n\n[Chorus]\nAnd it holds".to_owned(),
    };

    original.write(&dir).unwrap();
    let path: PathBuf = dir.join("song.md");
    println!("--- written to {} ---\n", path.display());
    print!("{}", std::fs::read_to_string(&path).unwrap());

    // Edit it the way a person would: change the caption, add a section.
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("68 BPM", "72 BPM")
        .replace(
            "[Chorus]\nAnd it holds",
            "[Chorus]\nAnd it holds\n\n[Outro]",
        );
    std::fs::write(&path, edited).unwrap();

    let back = studio::Recipe::read(&path).unwrap();
    println!("\n--- read back after hand-editing ---");
    println!("title    {}", back.title);
    println!("caption  {}", back.caption);
    println!(
        "lyrics   {} lines, ends {:?}",
        back.lyrics.lines().count(),
        back.lyrics.lines().last().unwrap_or("")
    );
    assert!(back.caption.contains("72 BPM"), "edit did not survive");
    assert!(
        back.lyrics.ends_with("[Outro]"),
        "added section did not survive"
    );
    assert_eq!(back.title, original.title, "colon in title did not survive");
    println!("\nround trip ok");
}
