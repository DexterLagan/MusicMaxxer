//! Request serialisation and client-side validation. SPEC §3.1, §4.

use minimax::validate::{self, Field};
use minimax::{
    AudioFormat, AudioSetting, Bitrate, CoverPreprocessRequest, GenerationRequest, Model,
    OutputFormat, ReferenceAudio, SampleRate,
};
use serde_json::Value;

fn body(req: &GenerationRequest) -> Value {
    serde_json::to_value(req).unwrap()
}

// ------------------------------------------------------------ serialisation

/// SPEC §3.1: the server default for output_format is `hex`. Never rely on it.
#[test]
fn output_format_and_stream_are_always_explicit() {
    let json = body(&GenerationRequest::new(
        Model::Music30Free,
        OutputFormat::Url,
    ));

    assert_eq!(json["output_format"], "url");
    assert_eq!(json["stream"], false);
}

#[test]
fn model_serialises_to_the_wire_id() {
    let json = body(&GenerationRequest::new(
        Model::Music30Free,
        OutputFormat::Hex,
    ));
    assert_eq!(json["model"], "music-3.0-free");
}

#[test]
fn unset_optional_fields_are_omitted_not_null() {
    // Sending `"lyrics": null` is a different request from omitting it.
    let json = body(&GenerationRequest::new(
        Model::Music30Free,
        OutputFormat::Url,
    ));

    for key in [
        "prompt",
        "lyrics",
        "audio_url",
        "audio_base64",
        "cover_feature_id",
    ] {
        assert!(json.get(key).is_none(), "{key} should be omitted");
    }
}

#[test]
fn audio_settings_serialise_as_integers() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url).audio_setting(
        AudioSetting::new(
            AudioFormat::Mp3,
            SampleRate::Hz44100,
            Some(Bitrate::Kbps256),
        ),
    );
    let json = body(&req);

    assert_eq!(json["audio_setting"]["sample_rate"], 44100);
    assert_eq!(json["audio_setting"]["bitrate"], 256000);
    assert_eq!(json["audio_setting"]["format"], "mp3");
}

#[test]
fn bitrate_is_dropped_for_formats_that_ignore_it() {
    // SPEC §3.1: mp3 only, inert for wav/pcm. Constructing through `new` means
    // a stale mp3 bitrate can't ride along on a wav request.
    let setting = AudioSetting::new(
        AudioFormat::Wav,
        SampleRate::Hz44100,
        Some(Bitrate::Kbps256),
    );
    assert!(setting.bitrate.is_none());

    let json =
        body(&GenerationRequest::new(Model::Music30Free, OutputFormat::Url).audio_setting(setting));
    assert!(json["audio_setting"].get("bitrate").is_none());
}

#[test]
fn default_audio_setting_is_wav_44100() {
    // SPEC §6.3: the user is producing masters, not previews.
    let json = body(&GenerationRequest::new(
        Model::Music30Free,
        OutputFormat::Url,
    ));
    assert_eq!(json["audio_setting"]["format"], "wav");
    assert_eq!(json["audio_setting"]["sample_rate"], 44100);
}

#[test]
fn undocumented_audio_values_are_rejected() {
    // The enums are closed on purpose — 48000 is plausible and not supported.
    assert!(SampleRate::from_value(48000).is_none());
    assert!(SampleRate::from_value(44100).is_some());
    assert!(Bitrate::from_value(320000).is_none());
}

#[test]
fn preprocess_request_carries_exactly_one_source() {
    let by_url = CoverPreprocessRequest::new(
        Model::MusicCoverFree,
        ReferenceAudio::Url("https://example.com/a.wav".into()),
    );
    let json = serde_json::to_value(&by_url).unwrap();
    assert_eq!(json["audio_url"], "https://example.com/a.wav");
    assert!(json.get("audio_base64").is_none());
}

// -------------------------------------------------------------- validation

fn fields(report: &minimax::Report) -> Vec<Field> {
    report.0.iter().map(|i| i.field).collect()
}

#[test]
fn a_complete_original_request_passes() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("Acid jazz, 104 BPM, E minor.")
        .lyrics("[Verse]\nSix flights up");

    assert!(validate::generation(&req).is_ok());
}

#[test]
fn caption_over_2000_chars_is_caught_before_sending() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("x".repeat(2001))
        .lyrics("[Verse]\nfine");

    let r = validate::generation(&req);
    assert!(!r.is_ok());
    assert!(fields(&r).contains(&Field::Prompt));
    // The message carries the measured value, per SPEC §4.
    assert!(r.to_string().contains("2001"));
}

#[test]
fn lyrics_over_3500_chars_is_caught() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("fine")
        .lyrics("x".repeat(3501));

    assert!(fields(&validate::generation(&req)).contains(&Field::Lyrics));
}

#[test]
fn limits_are_counted_in_characters_not_bytes() {
    // 2000 multi-byte characters is 2000 characters. Counting bytes would
    // reject a legal request and the UI counter would disagree with the client.
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("é".repeat(2000))
        .lyrics("[Verse]\nfine");

    assert!(validate::generation(&req).is_ok());
}

#[test]
fn non_instrumental_without_lyrics_is_refused() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url).prompt("a caption");
    assert!(fields(&validate::generation(&req)).contains(&Field::Lyrics));
}

#[test]
fn instrumental_needs_a_caption() {
    // SPEC §3.1: prompt is required when is_instrumental is true.
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url).instrumental(true);
    assert!(fields(&validate::generation(&req)).contains(&Field::Prompt));

    let ok = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("Cinematic trailer cue, C minor.")
        .instrumental(true);
    assert!(validate::generation(&ok).is_ok());
}

#[test]
fn auto_write_replaces_the_lyrics_requirement() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("a caption")
        .lyrics_optimizer(true);

    assert!(validate::generation(&req).is_ok());
}

#[test]
fn auto_write_with_lyrics_present_is_refused() {
    // SPEC §6.1: only enabled when the lyrics field is empty.
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("a caption")
        .lyrics("[Verse]\nmine")
        .lyrics_optimizer(true);

    assert!(!validate::generation(&req).is_ok());
}

#[test]
fn instrumental_and_auto_write_are_mutually_exclusive() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("a caption")
        .instrumental(true)
        .lyrics_optimizer(true);

    assert!(!validate::generation(&req).is_ok());
}

#[test]
fn reference_audio_on_a_non_cover_model_is_refused() {
    let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("a caption")
        .lyrics("[Verse]\nfine")
        .cover_feature_id("cf_1");

    assert!(fields(&validate::generation(&req)).contains(&Field::Model));
}

// ------------------------------------------------------------ cover-specific

#[test]
fn cover_prompt_bounds_differ_from_original() {
    // SPEC §3.1: 10–300 for covers, against 2000 for originals.
    let short = GenerationRequest::new(Model::MusicCoverFree, OutputFormat::Url)
        .prompt("too short")
        .audio_url("https://example.com/a.wav");
    assert!(fields(&validate::generation(&short)).contains(&Field::Prompt));

    let ok = GenerationRequest::new(Model::MusicCoverFree, OutputFormat::Url)
        .prompt("Slow gospel-soul rework, 68 BPM, Hammond organ.")
        .audio_url("https://example.com/a.wav");
    assert!(validate::generation(&ok).is_ok());
}

#[test]
fn cover_needs_exactly_one_reference_source() {
    // SPEC §4: exactly-one-of enforcement.
    let none = GenerationRequest::new(Model::MusicCoverFree, OutputFormat::Url)
        .prompt("Slow gospel-soul rework, 68 BPM.");
    assert!(fields(&validate::generation(&none)).contains(&Field::ReferenceAudio));

    let two = GenerationRequest::new(Model::MusicCoverFree, OutputFormat::Url)
        .prompt("Slow gospel-soul rework, 68 BPM.")
        .audio_url("https://example.com/a.wav")
        .audio_base64("AAAA");
    assert!(fields(&validate::generation(&two)).contains(&Field::ReferenceAudio));
}

#[test]
fn feature_id_flow_requires_lyrics() {
    // SPEC §3.1: when cover_feature_id is provided, lyrics is required.
    let req = GenerationRequest::new(Model::MusicCoverFree, OutputFormat::Url)
        .prompt("Slow gospel-soul rework, 68 BPM.")
        .cover_feature_id("cf_1");

    assert!(fields(&validate::generation(&req)).contains(&Field::Lyrics));
}

#[test]
fn reference_audio_bounds_report_the_measured_value() {
    // SPEC §4: "with the actual measured value in the message".
    let short = validate::reference_audio(4.5, 1_000);
    assert!(short.to_string().contains("4.5"), "got {short}");

    let long = validate::reference_audio(400.0, 1_000);
    assert!(!long.is_ok());

    let heavy = validate::reference_audio(60.0, 60 * 1024 * 1024);
    assert!(heavy.to_string().contains("60.0 MB"), "got {heavy}");

    assert!(validate::reference_audio(60.0, 5 * 1024 * 1024).is_ok());
}

#[test]
fn preprocess_rejects_non_cover_models() {
    // SPEC §3.2 takes only music-cover / music-cover-free.
    assert!(!validate::preprocess_model(Model::Music30Free).is_ok());
    assert!(validate::preprocess_model(Model::MusicCoverFree).is_ok());
}
