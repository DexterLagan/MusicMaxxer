//! App logic for MusicMaxxer — everything that is neither the API client nor
//! the UI.
//!
//! Kept out of the Tauri shell so it can be tested with `cargo test` rather
//! than through a webview. Currently step 2 of the build order in `SPEC.md`;
//! the library layout (§5), recipe file (§7) and lyric tools (§8) land here too.

pub mod keys;
pub mod library;
pub mod lyrics;
pub mod recipe;
pub mod settings;

pub use keys::{Credential, InMemoryStore, Keychain, SecretStore, StoreError};
pub use library::{Library, LibraryError, NewTake, RunRecord, StoredTake, TakeMeta};
pub use lyrics::{stray_tags, StrayTag, TAGS};
pub use recipe::{Recipe, RecipeError};
pub use settings::Settings;
