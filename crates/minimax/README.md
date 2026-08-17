# `minimax`

Client for MiniMax's hosted Music API. Step 1 of the build order in `../../SPEC.md`.

No Tauri, no filesystem, no UI — just typed requests, response parsing, error
mapping and client-side validation, so the whole surface is testable against
recorded fixtures.

```
src/
  model.rs      the six model IDs and their RPM figures (SPEC §3.1)
  request.rs    request bodies; closed enums for sample rate and bitrate
  response.rs   parsing, split from transport so fixtures can drive it
  error.rs      status codes → the sentences the user reads (SPEC §3.4)
  validate.rs   pre-flight checks (SPEC §4)
  client.rs     transport, timeouts, cancellation
tests/
  fixtures/     recorded response bodies
  wire.rs       parsing and error mapping
  requests.rs   serialisation and validation
  client.rs     key redaction, cancellation, validate-before-send
examples/
  smoke.rs      the live round-trip SPEC §11 requires
```

## Three things this crate refuses to get wrong

**HTTP 200 is not success.** Every response passes a `base_resp.status_code`
gate before `data` is touched. This is the API's normal failure shape, not an
edge case.

**There is no polling.** Generation is one long synchronous request with a 600s
timeout. Cancelling drops the request; there is no task ID to poll and no
partial state to reconcile.

**Nothing logs the key.** `Debug` on `Client` is hand-written to redact it, and
transport errors have the URL stripped. There is a test for each.

## Running the tests

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

43 tests, no network.

## Verifying against the live API

The tests prove the parser handles the shapes we recorded. They prove nothing
about whether our *request* shape is right — only the live API can:

```bash
export MINIMAX_API_KEY=...
cargo run -p minimax --example smoke
```

Costs one of three requests per minute on the free tier. Add
`-- --cancel-after 3` to check that cancelling mid-generation is clean.

## Known unknown

`CoverPreprocess::structure` is a `serde_json::Value`, not a typed struct. SPEC
§3.2 documents `structure_result` as a JSON string of segment types and
timestamps but not the field names inside it, and whether the timestamps are
seconds or milliseconds. Type it after seeing a live response — not before.
