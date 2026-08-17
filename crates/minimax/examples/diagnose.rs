//! Why is a `-free` model returning 1008?
//!
//! Prints the exact bytes we send and the raw bytes we get back, bypassing our
//! own parser so nothing is hidden behind a mapped message.
//!
//! Two probes, because one is not decisive:
//!
//!   A. Our real free-tier request.
//!   B. A deliberately invalid body (a model ID that does not exist).
//!
//! If B returns 2013 (invalid params), parameter validation runs *before* the
//! balance check — so the 1008 on A is a real verdict on a well-formed request,
//! and the fault is the account, not our JSON.
//!
//! If B *also* returns 1008, the balance gate runs first and rejects everything
//! regardless of shape — which would mean the account is unfunded and the
//! documented free tier is not reachable with this key.
//!
//! ```sh
//! cargo run -p minimax --example diagnose
//! ```
//!
//! Costs two of the three requests allowed per minute. Neither generates audio.

use minimax::{GenerationRequest, Model, OutputFormat};
use std::time::Duration;

const BASE: &str = "https://api.minimax.io/v1/music_generation";

#[tokio::main]
async fn main() {
    let Ok(key) = std::env::var("MINIMAX_API_KEY") else {
        eprintln!("Set MINIMAX_API_KEY first.");
        std::process::exit(2);
    };

    // Local, no network: MiniMax keys are JWTs. Report the shape only — never
    // the contents, and never the key itself.
    describe_key(&key);

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("http client");

    // ---- Probe A: the request the smoke test makes -------------------------
    let real = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt(
            "Lo-fi hip hop, 72 BPM, A minor. Dusty sampled Rhodes, vinyl crackle, \
             swung MPC drums, upright bass. No vocals.",
        )
        .instrumental(true);
    let body_a = serde_json::to_value(&real).expect("serialise");

    probe(&http, &key, "A · music-3.0-free, well-formed", &body_a).await;

    println!("\n   (pausing 21s — free tier allows 3 requests per minute)\n");
    tokio::time::sleep(Duration::from_secs(21)).await;

    // ---- Probe B: deliberately invalid, to order the two checks ------------
    let body_b = serde_json::json!({
        "model": "music-does-not-exist",
        "prompt": "probe",
        "stream": false,
        "output_format": "url",
        "is_instrumental": true
    });

    probe(&http, &key, "B · nonexistent model, malformed", &body_b).await;

    println!(
        "\nRead the two together:\n\
         \x20 B=2013, A=1008  → our JSON is fine; the account cannot reach the free tier.\n\
         \x20 B=1008, A=1008  → the balance gate precedes validation; account is unfunded.\n\
         \x20 A=0             → it works now; the earlier failure was transient.\n"
    );
}

async fn probe(http: &reqwest::Client, key: &str, label: &str, body: &serde_json::Value) {
    println!("── {label} ─────────────────────────────────");
    println!("POST {BASE}");
    println!(
        "request body:\n{}\n",
        serde_json::to_string_pretty(body).unwrap()
    );

    let res = http
        .post(BASE)
        .bearer_auth(key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(body)
        .send()
        .await;

    match res {
        Err(e) => println!("transport failed: {}\n", e.without_url()),
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();

            println!("HTTP {status}");

            // Show the envelope, trimmed — a success carries a huge audio blob.
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(mut v) => {
                    if let Some(audio) = v.pointer_mut("/data/audio") {
                        if let Some(s) = audio.as_str() {
                            *audio = serde_json::json!(format!("<{} chars elided>", s.len()));
                        }
                    }
                    println!(
                        "response body:\n{}\n",
                        serde_json::to_string_pretty(&v).unwrap()
                    );
                }
                Err(_) => println!("response body (not JSON):\n{text}\n"),
            }
        }
    }
}

/// Shape only. Never prints the key or any claim inside it.
///
/// No verdict on the format: we do not know what a MiniMax key is supposed to
/// look like, and an earlier version of this that guessed told a user their
/// valid key was malformed. Report the measurements; let the API judge.
fn describe_key(key: &str) {
    println!(
        "key         {} chars, {} underscore-separated parts, {} dot-separated",
        key.len(),
        key.split('_').count(),
        key.split('.').count()
    );
    if key.trim() != key {
        println!("            NOTE: has leading or trailing whitespace");
    }
    println!();
}
