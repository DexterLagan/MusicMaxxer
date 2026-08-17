//! Client behaviour that needs no network. SPEC §11.

use minimax::{Audio, Client, Error, GenerationRequest, Model, OutputFormat};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

/// Nothing is listening here, so any test that reaches transport will fail
/// loudly rather than silently passing for the wrong reason.
const DEAD: &str = "http://127.0.0.1:1";

fn client() -> Client {
    Client::with_base_url("sk-secret-key-value", DEAD).unwrap()
}

fn valid_request() -> GenerationRequest {
    GenerationRequest::new(Model::Music30Free, OutputFormat::Url)
        .prompt("Acid jazz, 104 BPM, E minor.")
        .instrumental(true)
}

/// SPEC §11: "Confirm neither API key appears in any log line, error message,
/// or file on disk outside the keychain."
#[test]
fn debug_output_does_not_leak_the_key() {
    let rendered = format!("{:?}", client());

    assert!(
        !rendered.contains("sk-secret-key-value"),
        "the key reached Debug output: {rendered}"
    );
    assert!(rendered.contains("<redacted>"));
}

#[tokio::test]
async fn transport_errors_do_not_carry_the_key() {
    // reqwest includes the URL in its Display by default; the key rides in a
    // header rather than the URL, but the error path strips the URL anyway.
    let err = client()
        .generate(&valid_request(), &CancellationToken::new())
        .await
        .unwrap_err();

    let rendered = err.to_string();
    assert!(!rendered.contains("sk-secret-key-value"), "{rendered}");
    assert!(matches!(err, Error::Transport(_)), "{err:?}");
}

/// SPEC §4: validation happens before anything is sent, so a malformed request
/// costs neither a round trip nor one of three requests per minute.
#[tokio::test]
async fn validation_runs_before_transport() {
    // Missing lyrics on a non-instrumental request. If this reached the network
    // it would come back as Transport against the dead address instead.
    let bad = GenerationRequest::new(Model::Music30Free, OutputFormat::Url).prompt("a caption");

    let err = client()
        .generate(&bad, &CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

/// SPEC §3.3: cancellation drops the request rather than polling for state.
#[tokio::test]
async fn an_already_cancelled_token_stops_before_sending() {
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = client()
        .generate(&valid_request(), &cancel)
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Cancelled), "{err:?}");
}

/// The real case: the request is genuinely in flight when the user hits Cancel.
/// A local listener that accepts and never answers stands in for a generation
/// that is still running, so the test does not depend on the network.
#[tokio::test]
async fn cancelling_mid_flight_returns_cancelled() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            open.push(sock); // hold it; never write a response
        }
    });

    let c = Client::with_base_url("sk-secret-key-value", format!("http://{addr}")).unwrap();
    let cancel = CancellationToken::new();
    let token = cancel.clone();

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        token.cancel();
    });

    let err = c.generate(&valid_request(), &cancel).await.unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err:?}");
}

// ------------------------------------------------------- downloading a take

/// A one-shot HTTP server that answers the first request with `body`, or with
/// `status` when it is not 200. Returns the URL to hit.
async fn serve_once(status: u16, body: Vec<u8>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut scratch = [0u8; 1024];
        let _ = sock.read(&mut scratch).await; // drain the request line

        let reason = if status == 200 { "OK" } else { "Error" };
        let head = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = sock.write_all(head.as_bytes()).await;
        let _ = sock.write_all(&body).await;
        let _ = sock.flush().await;
    });

    format!("http://{addr}/take.wav")
}

/// SPEC §6.4: hex responses are already bytes and must not be re-fetched.
#[tokio::test]
async fn fetching_hex_audio_is_a_passthrough() {
    let bytes = vec![0x52, 0x49, 0x46, 0x46];
    let got = client()
        .fetch_audio(&Audio::Bytes(bytes.clone()), &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(got, bytes);
}

#[tokio::test]
async fn a_url_take_is_downloaded_to_bytes() {
    let payload = b"RIFF....WAVEfmt ".to_vec();
    let url = serve_once(200, payload.clone()).await;

    let got = client()
        .fetch_audio(&Audio::Url(url), &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(got, payload);
}

/// The 24-hour expiry is the failure users will actually hit, so it gets its
/// own sentence rather than a CDN error page.
#[tokio::test]
async fn an_expired_link_says_so() {
    let url = serve_once(403, b"<xml>AccessDenied</xml>".to_vec()).await;

    let err = client()
        .fetch_audio(&Audio::Url(url), &CancellationToken::new())
        .await
        .unwrap_err();

    match err {
        Error::Http { status, body } => {
            assert_eq!(status, 403);
            assert!(body.contains("expired"), "got {body}");
            assert!(!body.contains("AccessDenied"), "leaked the CDN page");
        }
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_empty_download_is_not_a_valid_take() {
    let url = serve_once(200, Vec::new()).await;

    let err = client()
        .fetch_audio(&Audio::Url(url), &CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Decode(_)), "{err:?}");
}

#[tokio::test]
async fn a_download_can_be_cancelled() {
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = client()
        .fetch_audio(
            &Audio::Url("http://127.0.0.1:1/take.wav".to_owned()),
            &cancel,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Cancelled), "{err:?}");
}

#[test]
fn base_url_trailing_slash_does_not_double_up() {
    // A doubled slash would 404 on some gateways.
    let with_slash = Client::with_base_url("k", "https://api.minimax.io/").unwrap();
    assert!(format!("{with_slash:?}").contains("https://api.minimax.io\""));
}
