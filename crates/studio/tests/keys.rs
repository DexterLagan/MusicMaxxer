//! Credential handling. SPEC §2, §11.
//!
//! Runs against the in-memory store: the real keychain would prompt for access
//! on macOS, and these tests are about the logic around storage, not about
//! whether Apple's keychain works.

use studio::keys::{suspicious, InMemoryStore};
use studio::{Credential, SecretStore, StoreError};

fn store() -> InMemoryStore {
    InMemoryStore::new()
}

#[test]
fn a_key_round_trips() {
    let s = store();
    s.set(Credential::MiniMax, "abc.def.ghi").unwrap();

    assert_eq!(
        s.get(Credential::MiniMax).unwrap().as_deref(),
        Some("abc.def.ghi")
    );
    assert!(s.has(Credential::MiniMax));
}

#[test]
fn the_two_credentials_do_not_collide() {
    // Separate keychain services, so clearing one must not touch the other.
    let s = store();
    s.set(Credential::MiniMax, "minimax-key").unwrap();
    s.set(Credential::OpenRouter, "sk-openrouter").unwrap();

    s.delete(Credential::OpenRouter).unwrap();

    assert!(s.has(Credential::MiniMax));
    assert!(!s.has(Credential::OpenRouter));

    assert_ne!(
        Credential::MiniMax.service(),
        Credential::OpenRouter.service()
    );
}

#[test]
fn deleting_something_absent_is_not_an_error() {
    // Clearing the assistant's key when it was never set is a normal action,
    // not a failure to report to the user.
    assert!(store().delete(Credential::OpenRouter).is_ok());
}

#[test]
fn whitespace_is_trimmed_and_empty_is_refused() {
    let s = store();
    s.set(Credential::MiniMax, "  abc.def.ghi \n").unwrap();
    assert_eq!(
        s.get(Credential::MiniMax).unwrap().as_deref(),
        Some("abc.def.ghi")
    );

    assert!(matches!(
        s.set(Credential::MiniMax, "   "),
        Err(StoreError::Rejected(_))
    ));
}

/// The app must not pick a key up from the environment: the settings dialog is
/// the only way in. This test fails if someone reintroduces an env fallback.
#[test]
fn the_environment_is_never_consulted() {
    let var = "MINIMAX_API_KEY";
    let restore = std::env::var(var).ok();
    std::env::set_var(var, "env.key.value");

    let s = store();
    assert!(
        !s.has(Credential::MiniMax),
        "a key in the environment must not appear as stored"
    );
    assert_eq!(s.get(Credential::MiniMax).unwrap(), None);

    match restore {
        Some(v) => std::env::set_var(var, v),
        None => std::env::remove_var(var),
    }
}

// ----------------------------------------------------------- paste warnings

#[test]
fn paste_accidents_are_warned_about() {
    // A whole header pasted into the field — the specific diagnosis wins over
    // the generic "contains a space" one.
    assert!(suspicious("Bearer abc_def_ghi").unwrap().contains("Bearer"));

    // A copy that picked up a line break.
    assert!(suspicious("abc_def ghi").is_some());

    assert!(suspicious("   ").is_some());
}

/// Regression: an earlier version guessed at each vendor's key format and told
/// a user their valid MiniMax key looked malformed. Real keys are
/// underscore-separated; the check assumed dot-separated JWTs. We no longer
/// guess at formats at all, so any shape of key passes without complaint.
#[test]
fn no_vendor_format_is_assumed() {
    for key in [
        "abc_def_ghi",     // the real MiniMax shape
        "abc.def.ghi",     // a JWT
        "sk-or-v1-abcdef", // OpenRouter's convention
        "or-v1-abcdef",    // and something that is not
        "a-single-opaque-token",
    ] {
        assert_eq!(suspicious(key), None, "{key} should draw no warning");
    }
}

/// The warning is advisory. Only the API can say whether a key works, so an
/// odd-looking key must still be storable.
#[test]
fn an_odd_looking_key_can_still_be_saved() {
    let s = store();
    let odd = "???";

    assert!(s.set(Credential::MiniMax, odd).is_ok());
    assert!(s.has(Credential::MiniMax));
}
