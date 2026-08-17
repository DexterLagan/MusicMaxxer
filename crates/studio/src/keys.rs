//! Credential storage. SPEC §2 and §11.
//!
//! Two entries in the OS keychain, never a config file, never `localStorage`,
//! never a log line. The store is a trait so the app logic around it can be
//! tested without touching — or prompting for access to — the real keychain.
//!
//! **Keys arrive from the settings dialog and nowhere else.** The app does not
//! read the environment: an environment variable is a CLI convention, invisible
//! to anyone who does not live in a terminal, and it gives a key two possible
//! homes — which is how you end up debugging why the app still uses an old one.
//! The `examples/` binaries do read `MINIMAX_API_KEY`, because those genuinely
//! are command-line tools.

use std::collections::HashMap;
use std::sync::Mutex;

/// The two secrets this app holds. SPEC §2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Credential {
    /// Required. Without it the app cannot generate anything.
    MiniMax,
    /// Optional, absent by default. Only written when the user explicitly
    /// enables the lyric assistant (SPEC §8.5).
    OpenRouter,
}

impl Credential {
    pub const ALL: [Credential; 2] = [Credential::MiniMax, Credential::OpenRouter];

    /// Keychain service name. Stable — changing it orphans stored secrets.
    pub fn service(self) -> &'static str {
        match self {
            Credential::MiniMax => "com.minimaxmusicstudio.minimax",
            Credential::OpenRouter => "com.minimaxmusicstudio.openrouter",
        }
    }

    pub fn account(self) -> &'static str {
        "api-key"
    }

    pub fn label(self) -> &'static str {
        match self {
            Credential::MiniMax => "MiniMax API key",
            Credential::OpenRouter => "OpenRouter API key",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Could not reach the system keychain: {0}")]
    Backend(String),
    #[error("{0}")]
    Rejected(&'static str),
}

/// Somewhere secrets live. Implemented by the real keychain in the app and by
/// an in-memory map in tests.
///
/// Deliberately not `Debug`: a derived impl on an implementor is the easiest
/// way to print a secret by accident.
pub trait SecretStore: Send + Sync {
    fn get(&self, c: Credential) -> Result<Option<String>, StoreError>;
    fn set(&self, c: Credential, secret: &str) -> Result<(), StoreError>;
    fn delete(&self, c: Credential) -> Result<(), StoreError>;

    fn has(&self, c: Credential) -> bool {
        matches!(self.get(c), Ok(Some(_)))
    }
}

/// The real OS keychain.
pub struct Keychain;

impl Keychain {
    fn entry(c: Credential) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(c.service(), c.account())
            .map_err(|e| StoreError::Backend(e.to_string()))
    }
}

impl SecretStore for Keychain {
    fn get(&self, c: Credential) -> Result<Option<String>, StoreError> {
        match Self::entry(c)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(StoreError::Backend(e.to_string())),
        }
    }

    fn set(&self, c: Credential, secret: &str) -> Result<(), StoreError> {
        let trimmed = secret.trim();
        if trimmed.is_empty() {
            return Err(StoreError::Rejected("That key is empty"));
        }
        Self::entry(c)?
            .set_password(trimmed)
            .map_err(|e| StoreError::Backend(e.to_string()))
    }

    fn delete(&self, c: Credential) -> Result<(), StoreError> {
        match Self::entry(c)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(StoreError::Backend(e.to_string())),
        }
    }
}

/// For tests. Never wire this into a shipping build.
#[derive(Default)]
pub struct InMemoryStore(Mutex<HashMap<Credential, String>>);

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemoryStore {
    fn get(&self, c: Credential) -> Result<Option<String>, StoreError> {
        Ok(self.0.lock().unwrap().get(&c).cloned())
    }

    fn set(&self, c: Credential, secret: &str) -> Result<(), StoreError> {
        let trimmed = secret.trim();
        if trimmed.is_empty() {
            return Err(StoreError::Rejected("That key is empty"));
        }
        self.0.lock().unwrap().insert(c, trimmed.to_owned());
        Ok(())
    }

    fn delete(&self, c: Credential) -> Result<(), StoreError> {
        self.0.lock().unwrap().remove(&c);
        Ok(())
    }
}

// ------------------------------------------------------------- sanity checks

/// A soft check on a pasted key. Returns a warning to show *alongside* the
/// save, never a reason to refuse it.
///
/// **Only vendor-independent paste accidents are checked here.** An earlier
/// version guessed at each vendor's key format and warned when a key did not
/// match. It was wrong about MiniMax — real keys are underscore-separated, not
/// the dot-separated JWT the check assumed — and it told a user their perfectly
/// good key looked malformed.
///
/// That is the worse failure. A malformed key costs one round trip and comes
/// back as a clear `2049`; a false warning makes the app untrustworthy about
/// everything else it says. We have no reliable knowledge of these formats and
/// no way to keep up when a vendor changes one, so we do not pretend to.
///
/// (For the record, one MiniMax key seen on 2026-08-17 had three
/// underscore-separated parts. One sample, recorded, not enforced.)
pub fn suspicious(key: &str) -> Option<&'static str> {
    let k = key.trim();

    if k.is_empty() {
        return Some("That key is empty");
    }
    // Before the generic whitespace check: a pasted header also contains a
    // space, and "remove the Bearer prefix" is the more useful diagnosis.
    if k.to_ascii_lowercase().starts_with("bearer ") {
        return Some("Paste just the key, without the `Bearer` prefix");
    }
    if k.chars().any(char::is_whitespace) {
        return Some("That key contains a space — check it was copied whole");
    }

    None
}
