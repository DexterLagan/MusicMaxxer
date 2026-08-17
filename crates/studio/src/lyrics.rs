//! Structure tags, and catching bracketed text that will be sung.
//!
//! SPEC §3.1 documents exactly fourteen tags for the `lyrics` field. The list
//! is closed: anything else in square brackets is treated as lyric content and
//! **sung aloud**, including near-misses like `[Verse 1]` or a tag carrying a
//! performance note. Observed 2026-08-17 — the model sang
//! "Verse 1 - hushed breathy male tenor, close-miked, half-spoken, flat and
//! drained" as a line.
//!
//! That failure is expensive: it costs a full generation, minutes of waiting,
//! and one of three requests per minute, and it succeeds — so nothing in the
//! response says anything went wrong. Catching it before sending is the whole
//! point of this module.

use serde::Serialize;

/// The complete set from SPEC §3.1. Canonical — the UI builds its tag bar from
/// this list rather than keeping its own copy, so the two cannot drift.
pub const TAGS: [&str; 14] = [
    "[Intro]",
    "[Verse]",
    "[Pre Chorus]",
    "[Chorus]",
    "[Interlude]",
    "[Bridge]",
    "[Outro]",
    "[Post Chorus]",
    "[Transition]",
    "[Break]",
    "[Hook]",
    "[Build Up]",
    "[Inst]",
    "[Solo]",
];

/// A bracketed run that is not a recognised tag, and so will be sung.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StrayTag {
    /// Exactly as written, brackets included.
    pub text: String,
    /// 1-based, to match what the writer sees.
    pub line: usize,
    /// The tag they most likely meant, when it is guessable.
    pub suggestion: Option<String>,
}

/// Find bracketed text that the API will sing rather than treat as structure.
pub fn stray_tags(lyrics: &str) -> Vec<StrayTag> {
    let mut out = Vec::new();

    for (index, line) in lyrics.lines().enumerate() {
        for text in bracketed_runs(line) {
            if is_recognised(&text) {
                continue;
            }
            out.push(StrayTag {
                suggestion: suggest(&text),
                text,
                line: index + 1,
            });
        }
    }
    out
}

/// Whether the lyrics are safe to send without a warning.
pub fn is_clean(lyrics: &str) -> bool {
    stray_tags(lyrics).is_empty()
}

/// Case-insensitive, and tolerant of internal whitespace — `[pre chorus]` and
/// `[Pre  Chorus]` are the same tag. Everything else is a stray.
fn is_recognised(text: &str) -> bool {
    let normalised = normalise(text);
    TAGS.iter().any(|t| normalise(t) == normalised)
}

/// The recognised tag whose name opens this stray, if any. `[Verse 1 — …]`
/// suggests `[Verse]`; `[Whispered]` suggests nothing.
fn suggest(text: &str) -> Option<String> {
    let normalised = normalise(text);

    // Longest match first, so `[Post Chorus 2]` does not suggest `[Chorus]`.
    let mut candidates: Vec<&&str> = TAGS
        .iter()
        .filter(|t| {
            let name = normalise(t);
            normalised == name
                || normalised
                    .strip_prefix(&name)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
        .collect();

    candidates.sort_by_key(|t| std::cmp::Reverse(normalise(t).len()));
    candidates.first().map(|t| (**t).to_owned())
}

/// Lowercase, brackets stripped, runs of whitespace collapsed.
fn normalise(text: &str) -> String {
    let inner = text.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Every `[...]` run on a line. Unclosed brackets are ignored — an opening
/// bracket with no partner is ordinary text, not a broken tag.
fn bracketed_runs(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;

    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        out.push(format!("[{}]", &after[..close]));
        rest = &after[close + 1..];
    }
    out
}
