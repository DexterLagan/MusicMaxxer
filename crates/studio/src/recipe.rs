//! `song.md` — the portable recipe. SPEC §7.
//!
//! Markdown with YAML front matter: settings where a parser can reach them,
//! caption and lyrics where a person can read and edit them. Written at the
//! song root on every generate, and readable back to restore the whole form.
//!
//! The front matter is parsed by hand rather than with a YAML crate. The shape
//! is fixed and tiny — flat scalars plus two nested blocks — and the one rule
//! that matters (§7.2's parenthesised placeholder) is not something a general
//! YAML parser would know about. A hand-rolled reader also means a mangled file
//! degrades to "recover the writing" instead of failing outright.

use std::path::Path;

pub const FORMAT_VERSION: u32 = 1;
pub const FILENAME: &str = "song.md";

#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    #[error("Could not read that recipe: {0}")]
    Io(String),
    #[error("This recipe is version {found}; this app understands version {ours}")]
    Version { found: u32, ours: u32 },
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioSettings {
    pub format: String,
    pub sample_rate: u32,
    /// Written only for mp3, where it has an effect (SPEC §3.1).
    pub bitrate: Option<u32>,
}

/// A cover's reference, named by hash so a moved file can still be matched.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CoverRef {
    pub reference_file: String,
    pub reference_sha256: String,
    pub rights_confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Recipe {
    pub title: String,
    pub created: String,
    pub model: String,
    pub instrumental: bool,
    pub lyrics_optimizer: bool,
    pub audio: AudioSettings,
    pub cover: Option<CoverRef>,
    pub caption: String,
    pub lyrics: String,
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe {
            title: "Untitled song".to_owned(),
            created: String::new(),
            model: "music-3.0-free".to_owned(),
            instrumental: false,
            lyrics_optimizer: false,
            audio: AudioSettings {
                format: "wav".to_owned(),
                sample_rate: 44100,
                bitrate: None,
            },
            cover: None,
            caption: String::new(),
            lyrics: String::new(),
        }
    }
}

/// SPEC §7.2: a section holding one parenthesised line is a placeholder for
/// content we could not record, not content itself. Used when the model wrote
/// the lyrics and the API did not return them (§3.6).
pub const UNRECORDED_LYRICS: &str =
    "(written by the model — the API returns audio only, so these words are not recorded)";

impl Recipe {
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("---\n");
        s.push_str(&format!("minimax_music: {FORMAT_VERSION}\n"));
        s.push_str(&format!("title: {}\n", yaml_scalar(&self.title)));
        if !self.created.is_empty() {
            s.push_str(&format!("created: {}\n", self.created));
        }
        s.push_str(&format!("model: {}\n", self.model));
        s.push_str(&format!("instrumental: {}\n", self.instrumental));
        s.push_str(&format!("lyrics_optimizer: {}\n", self.lyrics_optimizer));
        s.push_str("audio:\n");
        s.push_str(&format!("  format: {}\n", self.audio.format));
        s.push_str(&format!("  sample_rate: {}\n", self.audio.sample_rate));
        // SPEC §7.1: bitrate appears only for mp3.
        if let Some(b) = self.audio.bitrate {
            if self.audio.format == "mp3" {
                s.push_str(&format!("  bitrate: {b}\n"));
            }
        }
        if let Some(c) = &self.cover {
            s.push_str("cover:\n");
            s.push_str(&format!(
                "  reference_file: {}\n",
                yaml_scalar(&c.reference_file)
            ));
            s.push_str(&format!("  reference_sha256: {}\n", c.reference_sha256));
            s.push_str(&format!("  rights_confirmed: {}\n", c.rights_confirmed));
        }
        s.push_str("---\n\n");

        s.push_str("## Caption\n\n");
        s.push_str(if self.caption.trim().is_empty() {
            "(none)"
        } else {
            self.caption.trim()
        });
        s.push_str("\n\n");

        if !self.instrumental || !self.lyrics.trim().is_empty() {
            s.push_str("## Lyrics\n\n");
            s.push_str(if self.lyrics.trim().is_empty() {
                if self.lyrics_optimizer {
                    UNRECORDED_LYRICS
                } else {
                    "(none)"
                }
            } else {
                self.lyrics.trim_end()
            });
            s.push('\n');
        }
        s
    }

    pub fn from_markdown(text: &str) -> Result<Recipe, RecipeError> {
        let (front, body) = split_front_matter(text);
        let map = parse_front_matter(front);

        if let Some(v) = map.get("minimax_music").and_then(|v| v.parse::<u32>().ok()) {
            if v > FORMAT_VERSION {
                return Err(RecipeError::Version {
                    found: v,
                    ours: FORMAT_VERSION,
                });
            }
        }

        let mut recipe = Recipe {
            title: map.get("title").cloned().unwrap_or_default(),
            created: map.get("created").cloned().unwrap_or_default(),
            model: map
                .get("model")
                .cloned()
                .unwrap_or_else(|| "music-3.0-free".to_owned()),
            instrumental: truthy(map.get("instrumental")),
            lyrics_optimizer: truthy(map.get("lyrics_optimizer")),
            audio: AudioSettings {
                format: map
                    .get("audio.format")
                    .cloned()
                    .unwrap_or_else(|| "wav".to_owned()),
                sample_rate: map
                    .get("audio.sample_rate")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(44100),
                bitrate: map.get("audio.bitrate").and_then(|v| v.parse().ok()),
            },
            cover: None,
            caption: section(body, "Caption").unwrap_or_default(),
            lyrics: section(body, "Lyrics").unwrap_or_default(),
        };

        if map.contains_key("cover.reference_sha256") || map.contains_key("cover.reference_file") {
            recipe.cover = Some(CoverRef {
                reference_file: map.get("cover.reference_file").cloned().unwrap_or_default(),
                reference_sha256: map
                    .get("cover.reference_sha256")
                    .cloned()
                    .unwrap_or_default(),
                rights_confirmed: truthy(map.get("cover.rights_confirmed")),
            });
        }

        if recipe.title.trim().is_empty() {
            recipe.title = "Untitled song".to_owned();
        }
        Ok(recipe)
    }

    pub fn write(&self, song_dir: &Path) -> Result<(), RecipeError> {
        std::fs::write(song_dir.join(FILENAME), self.to_markdown())
            .map_err(|e| RecipeError::Io(e.to_string()))
    }

    pub fn read(path: &Path) -> Result<Recipe, RecipeError> {
        let text = std::fs::read_to_string(path).map_err(|e| RecipeError::Io(e.to_string()))?;
        Recipe::from_markdown(&text)
    }
}

// -------------------------------------------------------------------- parsing

fn split_front_matter(text: &str) -> (&str, &str) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = t.strip_prefix("---") else {
        return ("", t);
    };
    let rest = rest.trim_start_matches(['\r', '\n']);
    match find_closing_fence(rest) {
        Some((front, body)) => (front, body),
        None => ("", t),
    }
}

/// The closing `---` must be alone on its line, so a horizontal rule inside the
/// body cannot be mistaken for the end of the front matter.
fn find_closing_fence(rest: &str) -> Option<(&str, &str)> {
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

/// Flattens the one level of nesting we use: `audio: { format: … }` becomes
/// the key `audio.format`.
fn parse_front_matter(front: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut block: Option<String> = None;

    for line in front.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value.trim());

        // Keys can contain digits — `reference_sha256` is the one that bites.
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }

        if indented {
            if let Some(parent) = &block {
                map.insert(format!("{parent}.{key}"), value);
            }
        } else if value.is_empty() {
            block = Some(key.to_owned());
        } else {
            block = None;
            map.insert(key.to_owned(), value);
        }
    }
    map
}

/// A `## Heading` section's body, or `None` when the heading is absent.
/// SPEC §7.2: a lone parenthesised line is a placeholder, so it reads as empty.
fn section(body: &str, name: &str) -> Option<String> {
    let mut lines = body.lines();
    let mut found = false;
    let mut out: Vec<&str> = Vec::new();

    for line in lines.by_ref() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("##") {
            let heading = heading.trim();
            if found {
                break; // the next section starts here
            }
            if heading.eq_ignore_ascii_case(name) {
                found = true;
            }
            continue;
        }
        if found {
            out.push(line);
        }
    }

    if !found {
        return None;
    }

    let text = out.join("\n").trim().to_owned();
    let placeholder = text.starts_with('(') && text.ends_with(')') && !text.contains('\n');
    Some(if placeholder { String::new() } else { text })
}

fn truthy(v: Option<&String>) -> bool {
    matches!(
        v.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("true" | "yes" | "on")
    )
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].replace("\\\"", "\"")
    } else {
        v.to_owned()
    }
}

/// Quote anything that would otherwise re-parse wrongly — a title containing a
/// colon is the realistic case.
fn yaml_scalar(s: &str) -> String {
    let needs_quotes = s.is_empty()
        || s.contains(':')
        || s.contains('#')
        || s.starts_with(['-', '[', '{', '"', '\'', ' '])
        || s.ends_with(' ');

    if needs_quotes {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_owned()
    }
}
