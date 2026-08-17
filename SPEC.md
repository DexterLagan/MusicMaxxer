# MusicMaxxer — build spec

A local desktop app (Rust + Tauri v2) for generating songs through MiniMax's
hosted Music API. Single user, single machine, no server component.

This spec is the contract. Where it states a limit or an enum, that value came
from the official API reference — do not "improve" it or substitute plausible
alternatives. Where it says TODO/verify, stop and ask rather than guessing.

**Revision 2** — folds in the decisions taken against the clickable UI sketch
(`design/ui-mockup.html`). Changed since revision 1: §2 (second keychain entry,
bundled pronunciation dictionary), §3.5 (new — auto-write behaviour), §5
(rewritten — one folder per song, take subfolders), §6.1 (song title, filterable
presets, lyric tools), §6.3 (appearance, tag embedding, lyric assistant), §6.4
(A/B compare dropped), §7 (new — the recipe file), §8 (new — lyric tools).
Design direction, build order and verification renumbered to §9–§11.

---

## 1. Scope

**In scope:** original song generation (`music-3.0`), cover generation from
reference audio (`music-cover`), the two-step cover preprocess workflow, a run
history with full reproducibility metadata, a portable recipe file per song, an
offline lyric-analysis toolkit, and a built-in player.

**Out of scope:** local/self-hosted model weights (this app only ever talks to
the hosted API), streaming output, any account/billing management, any cloud
sync, any telemetry.

**Optional and off by default:** the lyric assistant (§8.5) is the only feature
that contacts a service other than MiniMax. The app must be completely usable
with it disabled, and disabled is the shipped state.

---

## 2. Stack

- **Tauri v2**, Rust backend.
- **All HTTP happens in Rust** (`reqwest`), exposed to the frontend via Tauri
  commands. No API key may ever be readable from the webview context. Do not
  call any API from JavaScript.
- Frontend: your choice of a lightweight framework (Svelte or vanilla TS both
  fine). No heavy component library — see §9 for the design direction.
- **Two keychain entries** (`keyring` crate), never in a config file, never in
  `localStorage`, never logged:
  - `minimax_api_key` — required. Entered in the settings dialog and nowhere
    else. **The app never reads the environment for it.** An environment
    variable is a command-line convention: invisible to anyone not living in a
    terminal, and it gives the key two possible homes, which is how a stale one
    ends up being used without explanation. (The `examples/` binaries in
    `crates/minimax` do read `MINIMAX_API_KEY` — those are CLI tools.)
  - `openrouter_api_key` — optional, absent by default. Only written when the
    user explicitly enables the lyric assistant. Clearing the enable toggle
    deletes the entry.
- **Bundle the CMU Pronouncing Dictionary** as a compile-time asset (~134k
  entries, a few hundred KB). It backs §8 and must work with no network.
- Async runtime: tokio. Generation requests must be cancellable.

---

## 3. API contract

Base URL: `https://api.minimax.io`
Auth header: `Authorization: Bearer <API_KEY>`, `Content-Type: application/json`

### 3.1 `POST /v1/music_generation`

Request fields:

| Field | Type | Notes |
|---|---|---|
| `model` | string, **required** | See model table below |
| `prompt` | string | Style/mood description. Max 2000 chars. Required when `is_instrumental: true`. For cover models: **required, 10–300 chars** |
| `lyrics` | string | `\n`-separated. 1–3500 chars. Required for non-instrumental original generation. For cover models: optional, 10–1000 chars |
| `stream` | bool | Default false. **Leave false — out of scope** |
| `output_format` | `url` \| `hex` | **Server default is `hex`.** Always set explicitly |
| `audio_setting` | object | See below |
| `lyrics_optimizer` | bool | Default false. When true and `lyrics` empty, model writes lyrics from `prompt`. See §3.5 |
| `is_instrumental` | bool | Default false. When true, `lyrics` not required |
| `audio_url` | string | Cover models only. Mutually exclusive with `audio_base64` and `cover_feature_id` |
| `audio_base64` | string | Cover models only. Same exclusivity |
| `cover_feature_id` | string | Cover models only, two-step flow. When provided, `lyrics` is required |

`audio_setting`:
- `sample_rate`: `16000` | `24000` | `32000` | `44100`
- `bitrate`: `32000` | `64000` | `128000` | `256000` (mp3 only; inert for wav/pcm)
- `format`: `mp3` | `wav` | `pcm`

Model IDs:

| ID | Access | RPM |
|---|---|---|
| `music-3.0` | Token Plan / paid only | 120 |
| `music-3.0-free` | All users via API key | 3 |
| `music-2.6` | Token Plan / paid only | 120 |
| `music-2.6-free` | All users via API key | 3 |
| `music-cover` | Token Plan / paid only | 120 |
| `music-cover-free` | All users via API key | 3 |

Response:

```json
{
  "data": { "audio": "<hex string>", "status": 2 },
  "trace_id": "...",
  "extra_info": {
    "music_duration": 25364,
    "music_sample_rate": 44100,
    "music_channel": 2,
    "bitrate": 256000,
    "music_size": 813651
  },
  "base_resp": { "status_code": 0, "status_msg": "success" }
}
```

- `data.status`: `1` = in progress, `2` = completed.
- `data.audio` holds the hex-encoded file when `output_format: hex`; when `url`,
  it holds a link that **expires after 24 hours**.
- `music_duration` is in milliseconds.

### 3.2 `POST /v1/music_cover_preprocess`

Request: `model` (`music-cover` or `music-cover-free`) plus exactly one of
`audio_url` / `audio_base64`.

Response fields: `cover_feature_id` (valid 24 hours), `formatted_lyrics`
(ASR-extracted lyrics with section tags), `structure_result` (JSON *string*
containing segment types and timestamps — parse it, don't assume it's an object),
`audio_duration` (seconds).

This step is **free**. Identical audio returns the same feature ID via MD5
deduplication, so cache by content hash and skip redundant calls.

### 3.3 Critical behavioural notes

- **There is no task ID and no polling endpoint.** Generation is one long
  synchronous HTTP request. Set a generous client timeout (600s), and implement
  cancellation via a dropped request / `CancellationToken` — not by polling.
- **HTTP 200 does not mean success.** Always check `base_resp.status_code != 0`
  before touching `data`.
- Reference audio constraints: 6 seconds to 6 minutes, max 50 MB, common formats
  (mp3, wav, flac).

### 3.4 Error codes → user-facing messages

| Code | Message |
|---|---|
| `1002` | Rate limited. Free tier allows 3 requests/minute — retry shortly |
| `1004` | Authentication failed — check your API key |
| `1008` | Insufficient balance — add credit to the MiniMax account. The `-free` models are subject to the same check (see §3.5), so switching model will not help |
| `1026` | Content flagged as sensitive — revise the prompt or lyrics |
| `2013` | Invalid parameters |
| `2049` | Invalid API key |

Surface `trace_id` in a details/expander for any error, for support requests.

### 3.5 Observed behaviour that contradicts the documentation

Recorded against the live API on 2026-08-17 with `music-3.0-free`. Both items
are discrepancies, not workarounds — do not code around them silently.

**The free tier is not reachable on an unfunded account.** The model reference
states `music-3.0-free` is "Available to all users via API Key, with an RPM of
3", and the error reference lists `1008` as a balance issue only. In practice an
account with no payment method returns `1008` for `music-3.0-free`. Adding
credit resolved it immediately.

The ordering was established with two probes: a malformed request (nonexistent
model ID) returned `2013`, proving parameter validation runs *before* the
balance check — so the `1008` on a well-formed free-tier request was a genuine
account verdict rather than a request defect.

**Consequence for §3.4:** the `1008` message must not imply the fix is switching
to a `-free` model, because `-free` is subject to the same gate. Reword to point
at the account.

**Output duration is undocumented, and the `lyrics` field is what controls it —
through its section tags, not its words.** No documented request field sets
length. Measured on `music-3.0-free`:

| Request | Sections | Words | Audio out | Wall clock |
|---|---|---|---|---|
| `is_instrumental: true`, prompt only, empty lyrics | 0 | 0 | **2.8 s** | 43.7 s |
| `is_instrumental: true`, lyrics = section tags only | 8 | 0 | **86.8 s** | 106.3 s |
| Prompt + four-section lyric sheet | 6 | ~90 | **106.4 s** | 86.7 s |
| Prompt + full lyric sheet (from the app) | — | — | **240.0 s** | 191.8 s |
| Same 10-section sheet, caption A | 10 | 203 | **~181 s** | — |
| Same 10-section sheet, caption B | 10 | 203 | **~184 s** | — |
| Same 10-section sheet, caption C | 10 | 203 | **~193 s** | — |

`music_size` corroborated every reading to within a wav header, so these are
genuine file lengths.

Structure sets the floor and words extend it, but **only loosely**. The last
three rows are the same sheet — identical tags, identical words — sent with
three different captions, and they returned takes spanning about twelve
seconds. So the caption influences duration too, and no per-section constant
predicts a take within better than a few percent.

Any figure derived from section count is an estimate with real error bars.
**The app must not display a predicted duration**, for the same reason §9
refuses to predict wall-clock time.

**Consequences, both binding:**

1. **The structure tag bar is the app's duration control**, not a convenience.
   It is the only lever the user has over how long a take runs, and §6.1 must
   present it as such.
2. **`is_instrumental` must not disable the lyrics field.** A prompt-only
   instrumental returns roughly three seconds. An instrumental with a tag
   skeleton and no words returns a minute and a half. The field stays live as a
   *structure* field — see §6.1.

**Wall-clock time does not track output length.** An 86.8 s take needed 106.3 s
to produce; a *longer* 106.4 s take needed only 86.7 s. Observed range is
**44–192 s**, and queue or load appears to dominate. **Do not derive an ETA from
the requested structure** — §9's elapsed-time display stays honest by showing
elapsed time and a typical range, never a prediction.

**Unrecognised bracketed text is sung aloud.** SPEC §3.1 lists exactly fourteen
structure tags, and the list is closed. Anything else in square brackets is
treated as lyric content: observed 2026-08-17, the model sang
"Verse 1 - hushed breathy male tenor, close-miked, half-spoken, flat and
drained" as a line of the song.

Two things make this the worst failure mode in the app:

- **`[Verse 1]` is not a tag.** Neither is `[Chorus 2]`. Only the bare forms
  are recognised, so the most natural way to write a lyric sheet is wrong.
- **The request succeeds.** There is no error, no warning, and nothing in the
  response indicates it happened — the cost is a ruined take, three minutes of
  waiting, and one of three requests per minute.

The documentation does not state what happens to unrecognised brackets; this is
observed behaviour.

**Vocal and performance direction belongs in `prompt`, not `lyrics`.** The
guide's own example prompt carries it — "featuring a male vocalist with heavy
autotune" — alongside instrumentation.

**Required in the app (§6.1):** lint the lyrics field for bracketed runs that
are not exact tags, warn live as the user types, name the line, suggest the tag
they meant where it is guessable, and confirm before spending a generation.
Warn, never block — a writer may want a bracketed line sung.

**There is no four-minute ceiling — retracted 2026-08-17.** An earlier revision
of this section flagged the lone 240.0 s take as a possible cap and predicted
that fuller sheets would pin to it. Three subsequent takes from a *larger*
sheet (10 sections, 203 words, against 240.0 s's unrecorded but smaller one)
returned roughly 181 s, 184 s and 193 s. Nothing clusters at 240.

Two lessons, both worth keeping:

- **A round number is not evidence of a cap.** 240.0 s was one sample that
  happened to look meaningful. The prediction it produced was wrong three times
  in a row.
- **Longer sheets do not reliably produce longer takes.** The 240.0 s sheet was
  smaller than the ~185 s sheets. Whatever sets duration, section count alone
  does not order the outcomes.

The real ceiling, if one exists, is still unmeasured and is now above 240 s
only by assumption. Do not design a warning around a limit we have not seen.

### 3.6 Auto-write lyrics is reproducible only via ASR

`lyrics_optimizer` is a field on this same request — no second service and no
second key. But **the response returns audio only**: there is no field carrying
the words the model sang. An auto-written take is therefore the one thing in this
app that cannot be reproduced, and cannot be recorded in `song.md`.

Close the gap with the endpoint you already have. After the take lands, send the
finished track to `/v1/music_cover_preprocess` (§3.2, free) and take
`formatted_lyrics` — ASR'd, with section tags. Write those into the lyrics field
and into `song.md`.

Do this automatically after every take generated with `lyrics_optimizer: true`.
It costs nothing, and a silently unreproducible take defeats the point of the
library. Until recovery completes, `song.md` records the placeholder in §7.2.

---

## 4. Client-side validation

Validate **before** sending — a rejected request still costs a round trip and,
on the free tier, one of three requests per minute.

- Live character counters on Caption (2000) and Lyrics (3500), turning red past
  the limit with the submit button disabled.
- Cover mode: enforce the different limits (prompt 10–300, lyrics 10–1000).
- Reject reference audio outside 6s–6min or over 50 MB, with the actual measured
  value in the message.
- Exactly-one-of enforcement on `audio_url` / `audio_base64` / `cover_feature_id`.
- Client-side rate limiter matching the selected model's RPM (3 or 120), with a
  visible countdown when throttled rather than a silent stall. Exponential
  backoff on `1002`.

---

## 5. Library layout & reproducibility

**One folder per song. Takes live inside it.**

```
~/MiniMaxMusic/
  six-flights-up/
    song.md              ← the recipe: inputs, hand-editable, rewritten each generate
    take-01/
      track.wav
      run.json           ← the receipt: what happened, never hand-edited
    take-02/
      track.wav
      run.json
    take-04/
      track.wav
      run.json
```

### 5.1 Naming rules

- **Song folder** = the song title, slugified (lowercase, non-alphanumerics to
  single hyphens, trimmed, max 40 chars). Empty title → `untitled-song`.
- **Slug collisions** between two *different* titles get a numeric suffix —
  `third-coffee`, `third-coffee-2`. Silently; do not prompt.
- **Take folders** are `take-NN`, zero-padded to two digits. The next take number
  is `max(existing take numbers for this song) + 1`, so gaps from deleted takes
  are never reused.

### 5.2 Recipe vs receipt — keep them separate

These are two documents with opposite contracts. Do not merge them.

| | `song.md` | `run.json` |
|---|---|---|
| Holds | inputs only | what actually happened |
| Scope | one per song | one per take |
| Written | rewritten on every generate | once, at save |
| Edited by hand | **yes, expected** | **never** |
| Format | Markdown + YAML front matter (§7) | JSON |

The reason is not tidiness. If one file were both, hand-editing it to try take 12
would rewrite the provenance of take 11.

`run.json` captures the **complete** request payload (model, prompt, lyrics, all
audio settings, flags), the response `extra_info`, `trace_id`, an ISO-8601
timestamp, and the wall-clock duration of the call. For covers, also record the
reference audio's SHA-256, its original filename, and the rights acknowledgement
(§6.2) — never the audio itself.

This is the feature that makes the tool worth building rather than curling:
take 11 will be the good one, and you need to know exactly what produced it.

### 5.3 Take annotations — `meta.json`

A star rating is **mutable state about a take**, and it fits neither existing
file. `run.json` is the immutable receipt (§5.2) and must not be rewritten every
time you click a star; `song.md` is per-song inputs, and a rating is per-take
judgement. So a third small file, alongside the receipt:

```
take-04/
  track.wav
  run.json     ← receipt: written once, never edited
  meta.json    ← annotation: rewritten whenever you rate it
```

```json
{ "rating": 4, "rated_at": "2026-08-17T14:22:05Z" }
```

- `rating` is **0–5**, where **0 means unrated** and is the default. Absent file
  and `rating: 0` mean the same thing, so nothing is written until you rate.
- The rating lives **in the take folder, not in a central index**. An index is
  faster to read but drifts from the disk the moment a folder is moved or
  restored from backup, and §5's whole premise is that the folder is the truth.
  The history list already walks the tree to find takes.
- Deliberately not in the audio file's tags: re-rating would rewrite a
  multi-megabyte master to change one integer.

Three file kinds now, each with one job: **recipe** (`song.md`, inputs, per
song), **receipt** (`run.json`, what happened, per take, immutable),
**annotation** (`meta.json`, your judgement, per take, mutable).

### 5.4 History UI

Reverse-chronological, one row per take, filterable by model, by free text over
title/caption/lyrics, and **by minimum rating**. Each row shows the song title,
its take number, a one-line caption snippet, and a five-star control.

**Rating is a one-click action from the list.** The point of the feature is
sifting: generate a dozen takes, star them as they play, then filter to
"4 stars and up" and hear what survived. That means the stars are on the row
itself — not behind a detail view — and the rating filter sits beside the model
filter, defaulting to "any".

Each entry offers **Play**, **Clone to form** (repopulates every control,
carrying the title so the next generate becomes the next take of the same song),
and **Reveal in file manager**. Deletion asks for confirmation and only removes
the app's own directory — never touch files outside the library root.

---

## 6. Features by screen

### 6.1 Compose (primary screen)

In order down the page:

- **Song title** — a single-line field at the top, set in the page's largest
  type, no input chrome. Drives the folder name (§5.1), the history entry, the
  file tags, and `run.json`. Blank is legal and becomes `Untitled song`. Show the
  resolved destination path beside it, live — including the next take number.
- **Presets** — a filterable combo, not a plain dropdown. The filter matches
  across preset name, descriptor and caption text; arrow keys and Enter select.
  Ship 4–6 covering distinct genres, each written in the structured caption style
  the model responds to (genre, BPM, key, mood, vocal description, arrangement).
  Include "Blank" and "Last used". Presets are editable and stored as JSON in the
  app data dir so the user can add their own. Two footer actions in the popup:
  *Open a song.md recipe…* (§7.3) and *Edit presets.json…*
- **Instrumental** toggle → `is_instrumental`. Marks Caption as required, and
  **keeps the lyrics field live** — it becomes a structure field. Per §3.5,
  disabling it would cap every instrumental at about three seconds.
  - Relabel the field **Structure** while the toggle is on, and change its
    placeholder to say that sections set the length.
  - Word entry is not blocked — the model ignores lyrics under
    `is_instrumental` — but the tag bar is the expected input, and the syllable
    gutter (§8.1) hides itself when there are no words to count.
  - The two instrumental presets ship with a tag skeleton, never an empty
    field. A preset that produces a three-second clip is a broken preset.
- **Structure tag bar** — see the tag list below. This is the app's only control
  over take length (§3.5), so it is a primary control, not a convenience: it
  stays visible whenever the lyrics/structure field is editable.
- **Auto-write lyrics** toggle → `lyrics_optimizer`. Only enabled when the lyrics
  field is empty; mutually exclusive with Instrumental in the UI. When on, state
  plainly in the form that MiniMax writes the words, that they are not returned
  as text, and that the app will recover them via ASR after the take (§3.5).
- **Caption** textarea → `prompt`. Multi-line, autogrow.
- **Lyrics** textarea → `lyrics`. Fixed line-height, wrapping **off** (horizontal
  scroll for the rare long line) so the analysis gutter in §8 stays aligned to
  text lines.
- **Structure tag bar** above the lyrics field — click to insert at cursor:
  `[Intro]` `[Verse]` `[Pre Chorus]` `[Chorus]` `[Interlude]` `[Bridge]`
  `[Outro]` `[Post Chorus]` `[Transition]` `[Break]` `[Hook]` `[Build Up]`
  `[Inst]` `[Solo]`
- **Lyric tools row** — see §8.
- **Generate**, with `⌘↵`, and a `song.md…` action that shows the recipe the
  current form would write.

### 6.2 Cover (secondary tab)

Two-step flow, presented as two visible stages:

1. **Load reference** — file picker or URL. Runs `music_cover_preprocess`,
   labelled in the UI as free. Displays `audio_duration` and populates an
   editable lyrics box from `formatted_lyrics`. Render `structure_result`
   segments as a simple labelled timeline strip.
2. **Generate** — sends `cover_feature_id` + edited lyrics + style prompt.

Also offer the one-step path (reference audio + prompt, lyrics auto-extracted)
as a "Quick cover" option for when lyric editing isn't needed.

The tab carries its own **Song title** field, on the same rules as §6.1.

**Rights gate:** before the first cover generation in a session, require an
explicit checkbox confirming the user holds rights to the reference recording.
Store the acknowledgement in `run.json`. This is not decoration — the model
license prohibits infringing use, and the tool should make that a deliberate act
rather than an accident.

### 6.3 Settings

A dialog, reachable from the title bar. Output and model settings stay inline on
Compose in a collapsed panel whose summary shows their current state, so nothing
is ever secretly wrong.

**Output & model** (inline on Compose):
- Model/tier selector, showing the RPM difference inline.
- Format (`mp3`/`wav`/`pcm`), sample rate, bitrate. **Bitrate disabled unless
  mp3.** Default to `wav` @ 44100 — the user is producing masters, not previews.
- **Embed recipe in file tags** — checkbox, on by default, **disabled unless the
  format is mp3**. Copies `song.md` into an ID3 comment frame so a track that
  leaves the library still carries its recipe. Do not attempt this for wav or
  pcm: the tagging is fragile and DAWs strip it.
- Library directory picker.

**Settings dialog:**
- **API key** — MiniMax. Replace / clear, plus the `MINIMAX_API_KEY` import.
- **Lyric assistant** — optional, **off by default** (§8.5). An enable checkbox;
  the OpenRouter key field and model picker stay disabled until it is ticked.
  Unticking clears the stored key.
- **Appearance** — three radios: **System (default)**, Light, Dark. System means
  no stored override; follow the OS.

### 6.4 Output

- Inline player with waveform and scrubbing, plus Reveal.
- **No A/B compare.** Dropped deliberately — it added a second interaction model
  to the player for a workflow the user does not have.
- `output_format` is an internal decision, not a user control: request `url`,
  download immediately, fall back to `hex` decode (`bytes.from_hex`) if a URL
  isn't returned. Never leave the user holding an expiring link.
- On save, write an AI-generation note into `run.json`. If the format supports
  tags, also write it to the file. The user intends to distribute through
  DistroKid, where AI disclosure is both a platform requirement and a condition
  of the model license.

---

## 7. The recipe file — `song.md`

A portable, hand-editable record of the **inputs** that made a song. Written at
the song folder root, rewritten on every generate, and readable back to restore
the whole form. This is the ComfyUI-style workflow file for this app.

Markdown with YAML front matter, not JSON: settings need to be machine-parseable,
but lyrics need to be readable and editable in any text editor, and 3,500
characters of `\n`-escaped JSON string is miserable to work with. As `.md` it also
Quick Looks in Finder and renders on GitHub.

### 7.1 Format

```markdown
---
minimax_music: 1
title: Six Flights Up
created: 2026-08-16T14:22:05Z
model: music-3.0-free
instrumental: false
lyrics_optimizer: false
audio:
  format: mp3
  sample_rate: 44100
  bitrate: 256000
cover:
  reference_file: demo-take.wav
  reference_sha256: 9f2c4b71ae03d8c5f6b2…
  rights_confirmed: true
---

## Caption

Acid jazz, 104 BPM, E minor. Live kit with tight ghost notes, slap bass…

## Lyrics

[Verse]
Six flights up and the elevator's out
```

- `minimax_music` is the format version. Refuse a version you don't know rather
  than guessing at its shape.
- `bitrate` appears only when `format: mp3`.
- The `cover:` block appears only for cover generations.
- Quote any front-matter string containing a colon.
- Do **not** record lyric-analysis state (syllable counts, findings, which model
  checked the lyrics). The recipe is inputs, not editing history.

### 7.2 Parsing rules

- Front matter is delimited by `---` lines; parse with `serde_yaml`. Keys may
  contain digits (`reference_sha256`).
- Body sections are located by heading (`## Caption`, `## Lyrics`), so a mangled
  front matter still recovers the writing.
- A section whose entire content is a single parenthesised line is a
  **placeholder, not content** — treat it as empty. This is how an unrecovered
  auto-written take is recorded:
  `(written by the model — the API returns audio only, so these words are not recorded)`

### 7.3 Restoring from a recipe

Three ways in, all equivalent: drop a `song.md` on the window, the *Open a
song.md recipe…* action in the presets combo, or double-click the file once the
app owns the extension.

Restoring sets title, caption, lyrics, model, format, sample rate, bitrate, and
both flags. **A restore never fails because of a missing reference.** For a
cover recipe whose `reference_sha256` cannot be satisfied:

- Restore everything else, then flag the reference with two actions, *Locate
  file…* and *Continue without it*.
- Distinguish the two cases in the message. **Missing:** the named file is not in
  the library or at its recorded path. **Mismatched:** a file of that name exists
  but hashes differently — it has been re-encoded or edited since.
- A located file is re-hashed. A match reuses the cached `cover_feature_id`, so
  step 1 of the cover flow costs nothing again (§3.2).
- `cover_feature_id` itself expires after 24 hours; assume it is dead on restore
  and re-run preprocess from the re-hashed file.

---

## 8. Lyric tools

Three tools, in increasing order of what they require. **The first two need no
key, no network, and no model.** Only the third does, and it is off by default.

### 8.1 Syllables and rhyme — offline, always on

A quiet gutter to the right of the lyrics field, one row per text line, showing
the line's syllable count and its rhyme letter. Toggleable; on by default.

- **Syllables** come from the bundled dictionary (§2): one per stressed vowel in
  the phoneme string. Words absent from the dictionary fall back to a vowel-group
  heuristic and are marked approximate (`~9`, dimmed) so the user knows which
  numbers to trust.
- **Rhyme is compared as sound, not spelling.** Take each line's last word, take
  its rime — from the last stressed vowel to the end, stress marks stripped — and
  match phonetically. Letter-matching is not an acceptable substitute: it pairs
  *love* with *move* and fails to pair *eight* with *gate*.
- Rhyme letters are assigned **per section**, so `A` means the same thing
  locally. A line with no partner in its section shows `·`.
- Alignment is load-bearing: the gutter rows must track text lines exactly. Fixed
  line-height on both, and wrapping off in the textarea (§6.1).

### 8.2 Check — offline, always on

Three findings, all arithmetic over §8.1. No model involved.

| Finding | Condition |
|---|---|
| **meter** | Parallel lines across same-named sections differ by ≥ 3 syllables |
| **rhyme** | A line has no rhyme partner in a section where ≥ 2 others pair up |
| **phrasing** | A line exceeds 13 syllables — long for one breath at most tempos |

### 8.3 How findings are presented

- **No line numbers, and no line-number gutter.** A raw index is an
  implementation detail and the user would have to count to use it.
- Anchor the way a musician would say it: `Chorus 2 · line 2`, with the
  occurrence number included only when that section name repeats. Quote the line
  beneath the anchor.
- Hovering a finding highlights its row(s) in the gutter. Clicking the quoted
  line selects it in the editor.
- **Collapse identical findings.** A repeated chorus is one note, not one per
  repetition: findings sharing a kind and an identical line merge, the anchor
  names every place it lands, and applying a rewrite updates **all** of them so
  repeated sections cannot drift apart.

### 8.4 Enhance — requires the assistant

Proposes rewrites. **Never edits in place.** Opens a diff, one row per rewrite,
original struck through above the proposal, each row individually tickable, with
a single Undo available after applying. One rewrite per line maximum, even when a
line triggers two findings.

### 8.5 The lyric assistant — optional, off by default

The only outbound path to anything other than `api.minimax.io`, and it ships
disabled.

- Enabled by an explicit checkbox in Settings plus an OpenRouter API key. Until
  both are present, the Enhance control shows an `off` marker and activating it
  opens Settings rather than erroring.
- Model is chosen from a filterable list that also accepts any OpenRouter model
  ID typed in full. Default `anthropic/claude-sonnet-5`.
- When enabled, it adds **only the craft layer** to Check: imagery, cliché,
  prosody, whether a line earns its place. Those are judgement calls; §8.2 is
  arithmetic. Never let the model restate a measured finding.
- **Label the source.** Measured findings carry no attribution; model findings
  name the model that wrote them. The user must always be able to tell which is
  which.
- Measured findings render immediately; craft notes arrive when they arrive.
  Never make the offline results wait on the network.

---

## 9. Design direction

Minimalist and quiet. Two-pane: composition on the left, history on the right,
collapsible. Theme follows the system by default, overridable in Settings.

- One accent colour, used only for the generate action, active playback, and the
  rhyme letters. Semantic red/green for errors and free-step labels are separate
  from the accent and do not count as it.
- Generous whitespace; the title and the two text areas are the visual centre of
  gravity and should feel like a writing surface, not a form.
- Every secondary control lives in a collapsed panel — the default view shows
  Title, Caption, Lyrics, and Generate.
- No spinners without information: generation takes minutes, so show elapsed
  time, a cancel button, and say plainly that the API reports no progress.
- Type: one humanist sans throughout, monospace for structure tags, counters,
  the analysis gutter, timecodes and trace IDs.
- The lyric gutter and the findings list must stay recessive. They are reference,
  not the subject.

A clickable reference implementation of all of the above is in
`design/ui-mockup.html`. Where this spec and the sketch disagree, this spec wins.

---

## 10. Build order

1. Rust client for both endpoints + typed request/response structs + error
   mapping. Unit tests against recorded fixtures.
2. Keychain storage and MiniMax key onboarding.
3. Compose screen, original generation only, hardcoded settings. **Includes the
   settings dialog's API-key field** — with no environment import (§2), it is
   the only way a key can reach the app, so it cannot wait for step 6.
4. Library persistence in the §5 layout + `run.json` + history list +
   clone-to-form.
5. `song.md` write and read, drag-and-drop, file association.
   **File association still outstanding** — opening a `.md` by double-click
   needs a bundled `.app`, and bundling is deferred to step 10. Drag-and-drop
   is the working path until then.
6. Settings dialog, appearance, and validation.
7. Player.
8. Lyric tools §8.1–§8.3 — dictionary, gutter, measured checks. Offline only.
9. Cover tab, both flows, plus the missing/mismatched reference handling in §7.3.
10. Presets, ID3 embedding, auto-write ASR recovery (§3.5), polish.
11. Lyric assistant (§8.4–§8.5). **Last, deliberately.** Everything above must
    work with this step never built.

Ship 1–4 working before starting 5. Do not scaffold all eleven and leave stubs.

---

## 11. Verification

The developer cannot assume any request shape is correct until it round-trips
against the live API. Before declaring a stage done:

**API**
- Confirm a successful `music-3.0-free` instrumental generation writes a
  playable file.
- Confirm a `1008` and a `2049` are surfaced as the mapped messages, not as raw
  JSON or a panic.
- Confirm cancellation mid-generation leaves no partial file in the library.

**Keys**
- Confirm neither API key appears in any log line, error message, or file on disk
  outside the keychain.
- Confirm that with the lyric assistant disabled, the process opens no connection
  to any host other than `api.minimax.io`. Verify with a proxy or packet capture,
  not by reading the code.

**Library**
- Confirm two takes of one title land in `take-01` and `take-02` under a single
  song folder, and that deleting `take-01` does not cause the next take to reuse
  the number.
- Confirm two different titles that slugify identically produce `slug` and
  `slug-2`.

**Recipe**
- Confirm the full round trip: generate, hand-edit the resulting `song.md` in a
  text editor, drop it on the window, and verify every control returns to the
  edited state.
- Confirm a cover recipe whose reference audio has been deleted still restores
  everything else and flags the reference.

**Lyric tools**
- Confirm *love/glove* pair and *love/move* do not; confirm *eight*, *gate*,
  *weight* and *late* all pair. A build that gets these wrong is matching letters
  somewhere.
- Confirm the gutter stays aligned to text lines after inserting a structure tag
  mid-lyric.
- Confirm a rewrite applied to a repeated chorus updates every copy.

If any documented field behaves differently from this spec, stop and report the
discrepancy rather than working around it silently.
