//! Response parsing against recorded fixtures. SPEC §10 step 1.
//!
//! No network: every case here is a body we have seen (or expect) from the API,
//! parsed through the same code path the client uses.

use minimax::{parse_cover_preprocess, parse_generation, Error, JobStatus, Model, OutputFormat};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

// ---------------------------------------------------------------- generation

#[test]
fn hex_payload_is_decoded_to_bytes() {
    let g = parse_generation(&fixture("generation_hex"), OutputFormat::Hex).unwrap();

    // "494433040000000000" is an ID3 header — the point is that we hand back
    // bytes, not the hex string.
    assert_eq!(
        g.audio.as_bytes().unwrap(),
        &[0x49, 0x44, 0x33, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00]
    );
    assert_eq!(g.status, JobStatus::Completed);
}

#[test]
fn url_payload_is_passed_through() {
    let g = parse_generation(&fixture("generation_url"), OutputFormat::Url).unwrap();

    assert!(g.audio.as_url().unwrap().starts_with("https://"));
    assert!(g.audio.as_bytes().is_none());
}

#[test]
fn music_duration_stays_in_milliseconds() {
    // SPEC §3.1. Reading this as seconds would put a 2:32 take at 42 hours.
    let g = parse_generation(&fixture("generation_url"), OutputFormat::Url).unwrap();

    assert_eq!(g.extra.music_duration, 152_000);
    assert!((g.extra.duration_secs() - 152.0).abs() < f64::EPSILON);
}

#[test]
fn in_progress_status_is_distinguished_from_completed() {
    let g = parse_generation(&fixture("generation_in_progress"), OutputFormat::Hex).unwrap();
    assert_eq!(g.status, JobStatus::InProgress);
}

#[test]
fn missing_extra_info_defaults_rather_than_failing() {
    // The in-progress fixture has no `extra_info`; a take without metadata is
    // still a take.
    let g = parse_generation(&fixture("generation_in_progress"), OutputFormat::Hex).unwrap();
    assert_eq!(g.extra.music_duration, 0);
}

#[test]
fn odd_length_hex_is_reported_not_truncated() {
    let body = br#"{"data":{"audio":"4944","status":2},"base_resp":{"status_code":0}}"#;
    assert!(parse_generation(body, OutputFormat::Hex).is_ok());

    let bad = br#"{"data":{"audio":"494","status":2},"base_resp":{"status_code":0}}"#;
    match parse_generation(bad, OutputFormat::Hex) {
        Err(Error::Decode(m)) => assert!(m.contains("odd number")),
        other => panic!("expected a decode error, got {other:?}"),
    }
}

#[test]
fn success_without_audio_is_an_error() {
    let body = br#"{"data":{"audio":"","status":2},"base_resp":{"status_code":0}}"#;
    assert!(matches!(
        parse_generation(body, OutputFormat::Url),
        Err(Error::Decode(_))
    ));
}

// -------------------------------------------------------------- error mapping

/// SPEC §3.3: HTTP 200 with a non-zero status_code is the common failure shape.
#[test]
fn http_200_with_error_code_is_an_error() {
    let err = parse_generation(&fixture("error_1008"), OutputFormat::Url).unwrap_err();
    assert_eq!(err.code(), Some(1008));
}

#[test]
fn mapped_messages_match_the_spec() {
    let cases = [
        ("error_1008", 1008, "Insufficient balance"),
        ("error_2049", 2049, "Invalid API key"),
        ("error_1002", 1002, "Rate limited"),
    ];

    for (name, code, expected_prefix) in cases {
        let err = parse_generation(&fixture(name), OutputFormat::Url).unwrap_err();
        assert_eq!(err.code(), Some(code), "{name}");
        assert!(
            err.to_string().starts_with(expected_prefix),
            "{name}: got {err}"
        );
    }
}

#[test]
fn every_documented_code_has_its_own_message() {
    // SPEC §3.4. Guards against a copy-paste that maps two codes to one string.
    let codes = [1002, 1004, 1008, 1026, 2013, 2049];
    let mut seen = Vec::new();
    for c in codes {
        let m = minimax::user_message(c, "server wording");
        assert!(!m.contains("server wording"), "{c} fell through to default");
        assert!(!seen.contains(&m), "{c} duplicates another code's message");
        seen.push(m);
    }
}

#[test]
fn unmapped_code_falls_back_to_server_wording() {
    let err = parse_generation(&fixture("error_unknown_code"), OutputFormat::Url).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("9001"), "got {text}");
    assert!(
        text.contains("region temporarily unavailable"),
        "got {text}"
    );
}

/// SPEC §3.4: surface `trace_id` on any error, for support requests.
#[test]
fn trace_id_survives_onto_the_error() {
    let err = parse_generation(&fixture("error_1008"), OutputFormat::Url).unwrap_err();
    assert_eq!(err.trace_id(), Some("5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d"));
}

#[test]
fn only_rate_limiting_is_retryable() {
    let limited = parse_generation(&fixture("error_1002"), OutputFormat::Url).unwrap_err();
    let broke = parse_generation(&fixture("error_2049"), OutputFormat::Url).unwrap_err();

    assert!(limited.is_retryable());
    assert!(!broke.is_retryable(), "retrying a bad key just burns quota");
}

// ---------------------------------------------------------------- preprocess

#[test]
fn preprocess_parses_the_structure_string() {
    // SPEC §3.2: `structure_result` is a JSON *string*, not an object. Treating
    // it as an object is the mistake this test exists to catch.
    let p = parse_cover_preprocess(&fixture("cover_preprocess")).unwrap();

    assert_eq!(p.cover_feature_id, "cf_9f2c4b71ae03d8c5f6b2");
    assert!(p.formatted_lyrics.starts_with("[Verse]"));

    let segments = p
        .structure
        .as_array()
        .expect("structure parsed to an array");
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0]["type"], "intro");

    // The raw string is kept verbatim for run.json.
    assert!(p.structure_raw.starts_with('['));
}

#[test]
fn preprocess_duration_is_seconds() {
    // SPEC §3.2 — unlike music_duration, which is milliseconds.
    let p = parse_cover_preprocess(&fixture("cover_preprocess")).unwrap();
    assert!((p.audio_duration - 222.4).abs() < 0.001);
}

#[test]
fn preprocess_errors_go_through_the_same_gate() {
    let err = parse_cover_preprocess(&fixture("error_2049")).unwrap_err();
    assert_eq!(err.code(), Some(2049));
}

#[test]
fn unparseable_structure_string_is_reported() {
    let body = br#"{"cover_feature_id":"cf_1","structure_result":"not json","base_resp":{"status_code":0}}"#;
    match parse_cover_preprocess(body) {
        Err(Error::Decode(m)) => assert!(m.contains("structure_result")),
        other => panic!("expected a decode error, got {other:?}"),
    }
}

// -------------------------------------------------------------------- models

#[test]
fn model_ids_and_rpm_match_the_spec_table() {
    // SPEC §3.1. These figures drive the client-side limiter; a wrong one
    // means silent throttling or burned quota.
    let expected = [
        (Model::Music30, "music-3.0", 120, false),
        (Model::Music30Free, "music-3.0-free", 3, false),
        (Model::Music26, "music-2.6", 120, false),
        (Model::Music26Free, "music-2.6-free", 3, false),
        (Model::MusicCover, "music-cover", 120, true),
        (Model::MusicCoverFree, "music-cover-free", 3, true),
    ];

    for (model, id, rpm, is_cover) in expected {
        assert_eq!(model.as_str(), id);
        assert_eq!(model.rpm(), rpm, "{id}");
        assert_eq!(model.is_cover(), is_cover, "{id}");
        assert_eq!(id.parse::<Model>().unwrap(), model);
    }
}

#[test]
fn free_equivalent_is_what_we_suggest_after_1008() {
    assert_eq!(Model::Music30.free_equivalent(), Model::Music30Free);
    assert_eq!(Model::MusicCover.free_equivalent(), Model::MusicCoverFree);
    assert_eq!(Model::Music26Free.free_equivalent(), Model::Music26Free);
}
