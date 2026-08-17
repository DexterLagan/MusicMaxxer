//! Client for MiniMax's hosted Music API.
//!
//! Step 1 of the build order in `SPEC.md`: typed requests and responses, error
//! mapping, and client-side validation, with no Tauri and no filesystem so the
//! whole surface is testable against recorded fixtures.
//!
//! Two things about this API are easy to get wrong, and both are enforced here:
//!
//! - **HTTP 200 does not mean success** (SPEC §3.3). Every response goes through
//!   a `base_resp.status_code` gate before `data` is touched.
//! - **There is no task ID and no polling endpoint.** Generation is one long
//!   synchronous request; cancellation drops it rather than polling for state.
//!
//! ```no_run
//! use minimax::{Client, GenerationRequest, Model, OutputFormat};
//! use tokio_util::sync::CancellationToken;
//!
//! # async fn run() -> Result<(), minimax::Error> {
//! let client = Client::new(std::env::var("MINIMAX_API_KEY").unwrap())?;
//!
//! let req = GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
//!     .prompt("Acid jazz, 104 BPM, E minor. Live kit, slap bass, Rhodes.")
//!     .instrumental(true);
//!
//! let take = client.generate(&req, &CancellationToken::new()).await?;
//! println!("{:.1}s", take.extra.duration_secs());
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod error;
pub mod model;
pub mod request;
pub mod response;
pub mod validate;

pub use client::{
    Client, DEFAULT_BASE_URL, DOWNLOAD_TIMEOUT, GENERATE_TIMEOUT, PREPROCESS_TIMEOUT,
};
pub use error::{user_message, Error};
pub use model::Model;
pub use request::{
    AudioFormat, AudioSetting, Bitrate, CoverPreprocessRequest, GenerationRequest, OutputFormat,
    ReferenceAudio, SampleRate,
};
pub use response::{
    parse_cover_preprocess, parse_generation, Audio, BaseResp, CoverPreprocess, ExtraInfo,
    Generation, JobStatus,
};
pub use validate::{Field, Issue, Report};
