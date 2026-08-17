//! Live round-trip against the real API. SPEC §11.
//!
//! The unit tests use recorded fixtures and prove nothing about whether our
//! request shape is actually correct. This does — and it is the only thing that
//! can. Costs one of three requests per minute on the free tier.
//!
//! ```sh
//! export MINIMAX_API_KEY=...
//! cargo run -p minimax --example smoke
//! cargo run -p minimax --example smoke -- --cancel-after 3
//! ```

use minimax::{Client, Error, GenerationRequest, Model, OutputFormat};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let Ok(key) = std::env::var("MINIMAX_API_KEY") else {
        eprintln!("Set MINIMAX_API_KEY first. This example makes a real, billable request.");
        std::process::exit(2);
    };

    // Optional: prove cancellation leaves nothing behind (SPEC §11).
    let cancel_after = std::env::args()
        .skip_while(|a| a != "--cancel-after")
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok());

    let client = match Client::new(key) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Could not build the client: {e}");
            std::process::exit(1);
        }
    };

    // SPEC §11: "a successful music-3.0-free instrumental generation writes a
    // playable file". Instrumental keeps it to one field and avoids spending
    // the request on a lyrics mistake.
    //
    // `--lyrics` sends a structured lyric sheet instead. Observed 2026-08-17:
    // the instrumental form returned 2.8s of audio. Nothing in the documented
    // request controls length, so the open question is whether lyrics are what
    // drive it. This flag is the experiment.
    let with_lyrics = std::env::args().any(|a| a == "--lyrics");

    // `--inst-tags`: instrumental, but with a structure-tag-only lyric sheet.
    // The open question after 2026-08-17: prompt-only instrumentals return ~3s,
    // so is it *lyrics* that drive length, or *structure*? If tags alone extend
    // an instrumental, that is how the app must build every instrumental take.
    let inst_tags = std::env::args().any(|a| a == "--inst-tags");

    let req = if inst_tags {
        GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
            .prompt(
                "Cinematic trailer cue, 90 BPM building to 140, C minor. Sub hits, \
                 taiko ostinato, brass clusters, high sustained strings. Fully instrumental.",
            )
            .instrumental(true)
            .lyrics(
                "[Intro]\n\n[Build Up]\n\n[Inst]\n\n[Break]\n\n\
                 [Build Up]\n\n[Inst]\n\n[Solo]\n\n[Outro]",
            )
    } else if with_lyrics {
        GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
            .prompt(
                "Lo-fi hip hop, 72 BPM, A minor. Dusty sampled Rhodes, vinyl crackle, \
                 swung MPC drums, upright bass. Female lead, close and unhurried.",
            )
            .lyrics(
                "[Intro]\n\n\
                 [Verse]\nSix flights up and the elevator's out\n\
                 The city keeps its own hours anyway\n\
                 I count the neon like it owes me rent\n\n\
                 [Chorus]\nTill the morning gets it right\n\
                 Six more hours in the dark\n\
                 And I'll be gone before the light\n\n\
                 [Verse]\nCold coffee and a radio playing static\n\
                 Somebody's window throws a square of gold\n\
                 I take the stairs, I never take the lift\n\n\
                 [Chorus]\nTill the morning gets it right\n\
                 Six more hours in the dark\n\
                 And I'll be gone before the light\n\n\
                 [Outro]",
            )
    } else {
        GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
            .prompt(
                "Lo-fi hip hop, 72 BPM, A minor. Dusty sampled Rhodes, vinyl crackle, \
                 swung MPC drums, upright bass. No vocals.",
            )
            .instrumental(true)
    };

    let cancel = CancellationToken::new();
    if let Some(secs) = cancel_after {
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs)).await;
            eprintln!("  … cancelling after {secs}s");
            token.cancel();
        });
    }

    println!("model      {}", req.model);
    println!("rpm        {}", req.model.rpm());
    println!(
        "mode       {}",
        if inst_tags {
            "instrumental + structure tags, no words"
        } else if with_lyrics {
            "lyrics, 4 sections"
        } else {
            "instrumental, prompt only"
        }
    );
    println!("waiting    the API returns nothing until the track is done\n");

    let started = Instant::now();
    let result = client.generate(&req, &cancel).await;
    let elapsed = started.elapsed();

    match result {
        Ok(take) => {
            println!("ok         in {:.1}s", elapsed.as_secs_f64());
            println!("status     {:?}", take.status);
            println!("duration   {:.1}s", take.extra.duration_secs());
            println!(
                "audio      {}",
                match &take.audio {
                    minimax::Audio::Url(u) => format!("url, {} chars (expires in 24h)", u.len()),
                    minimax::Audio::Bytes(b) => format!("{} bytes", b.len()),
                }
            );
            println!("trace_id   {}", take.trace_id.as_deref().unwrap_or("—"));

            // Cross-check duration against size, so a mis-scaled unit or a
            // truncated field can't be mistaken for a genuinely short track.
            let bytes_per_sec = take.extra.music_sample_rate * take.extra.music_channel * 2;
            if bytes_per_sec > 0 {
                let implied = take.extra.music_size as f64 / bytes_per_sec as f64;
                println!(
                    "size check {:.2}s implied by music_size at {} Hz × {}ch × 16-bit",
                    implied, take.extra.music_sample_rate, take.extra.music_channel
                );
            }

            println!("\nextra_info {:#?}", take.extra);
        }
        Err(Error::Cancelled) => {
            println!(
                "cancelled  after {:.1}s — nothing written",
                elapsed.as_secs_f64()
            );
        }
        Err(e) => {
            // This is the path SPEC §11 wants checked for 1008 and 2049: a
            // mapped sentence, not raw JSON and not a panic.
            println!("failed     in {:.1}s", elapsed.as_secs_f64());
            println!("message    {e}");
            if let Some(code) = e.code() {
                println!("code       {code}");
            }
            println!("trace_id   {}", e.trace_id().unwrap_or("—"));
            std::process::exit(1);
        }
    }
}
