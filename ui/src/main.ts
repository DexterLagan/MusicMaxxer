// Compose screen. SPEC §6.1, build order step 3.
//
// This file never sees an API key and never touches the network: every effect
// goes through a Rust command.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";

// ------------------------------------------------------------------- types

interface KeyStatus {
  minimax: boolean;
}

/** One stored take, as `studio::StoredTake` serialises it. */
interface Take {
  song_slug: string;
  title: string;
  take: number;
  dir: string;
  audio_path: string;
  duration_secs: number;
  model: string;
  caption: string;
  created_at: string;
  call_secs: number;
  rating: number;
}

interface TakeResult extends Take {
  trace_id: string | null;
  bytes: number;
}

interface FieldIssue {
  field: string;
  message: string;
}

interface Failure {
  message: string;
  code: number | null;
  traceId: string | null;
  cancelled: boolean;
  fields: FieldIssue[];
}

/** One selectable model, as `available_models` returns it. */
interface ModelInfo {
  id: string;
  rpm: number;
  free_tier: boolean;
}

/** The whole output-panel state in one shape, as `get_settings`/`save_settings` return it. */
interface SettingsPayload {
  model: string;
  format: string;
  sample_rate: number;
  bitrate: number | null;
  library_root: string;
  is_custom_library_root: boolean;
}

/** `studio::Recipe` — the parsed contents of a song.md. */
interface Recipe {
  title: string;
  created: string;
  model: string;
  instrumental: boolean;
  lyrics_optimizer: boolean;
  audio: { format: string; sample_rate: number; bitrate: number | null };
  cover: { reference_file: string; reference_sha256: string; rights_confirmed: boolean } | null;
  caption: string;
  lyrics: string;
}

function toFailure(raw: unknown): Failure {
  const r = (raw ?? {}) as Record<string, unknown>;
  if (typeof raw === "string") {
    return { message: raw, code: null, traceId: null, cancelled: false, fields: [] };
  }
  return {
    message: String(r.message ?? "Something went wrong"),
    code: (r.code as number | null) ?? null,
    traceId: (r.trace_id as string | null) ?? null,
    cancelled: Boolean(r.cancelled),
    fields: ((r.fields as FieldIssue[]) ?? []).map((f) => ({
      field: String(f.field),
      message: String(f.message),
    })),
  };
}

// ------------------------------------------------------------------- setup

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing element #${id}`);
  return el as T;
};

const els = {
  title: $<HTMLInputElement>("songTitle"),
  caption: $<HTMLTextAreaElement>("caption"),
  lyrics: $<HTMLTextAreaElement>("lyrics"),
  lyricsLabel: $<HTMLLabelElement>("lyricsLabel"),
  lengthHint: $<HTMLSpanElement>("lengthHint"),
  capCount: $<HTMLSpanElement>("capCount"),
  lyrCount: $<HTMLSpanElement>("lyrCount"),
  tags: $<HTMLDivElement>("tags"),
  strayWarn: $<HTMLDivElement>("strayWarn"),
  swInst: $<HTMLButtonElement>("swInst"),
  genBtn: $<HTMLButtonElement>("genBtn"),
  cancelBtn: $<HTMLButtonElement>("cancelBtn"),
  genPanel: $<HTMLDivElement>("genPanel"),
  elapsed: $<HTMLSpanElement>("elapsed"),
  notices: $<HTMLDivElement>("notices"),
  keyPill: $<HTMLSpanElement>("keyPill"),
  keyPillText: $<HTMLSpanElement>("keyPillText"),
  themeBtn: $<HTMLButtonElement>("themeBtn"),
  themeIcon: document.getElementById("themeIcon") as unknown as SVGSVGElement,
  settingsBtn: $<HTMLButtonElement>("settingsBtn"),
  scrim: $<HTMLDivElement>("scrim"),
  keyInput: $<HTMLInputElement>("keyInput"),
  keyWarn: $<HTMLDivElement>("keyWarn"),
  modalSave: $<HTMLButtonElement>("modalSave"),
  modalCancel: $<HTMLButtonElement>("modalCancel"),
  clearKey: $<HTMLButtonElement>("clearKey"),
  histList: $<HTMLDivElement>("histList"),
  histCount: $<HTMLSpanElement>("histCount"),
  histSearch: $<HTMLInputElement>("histSearch"),
  histRating: $<HTMLSelectElement>("histRating"),
  player: $<HTMLDivElement>("player"),
  playBtn: $<HTMLButtonElement>("playBtn"),
  playIcon: document.getElementById("playIcon") as unknown as SVGSVGElement,
  playName: $<HTMLSpanElement>("playName"),
  playPos: $<HTMLSpanElement>("playPos"),
  playDur: $<HTMLSpanElement>("playDur"),
  seek: $<HTMLInputElement>("seek"),
  revealBtn: $<HTMLButtonElement>("revealBtn"),
  outputToggle: $<HTMLButtonElement>("outputToggle"),
  outputPanel: $<HTMLDivElement>("outputPanel"),
  modelSelect: $<HTMLSelectElement>("modelSelect"),
  formatSelect: $<HTMLSelectElement>("formatSelect"),
  sampleRateSelect: $<HTMLSelectElement>("sampleRateSelect"),
  bitrateSelect: $<HTMLSelectElement>("bitrateSelect"),
  libraryPath: $<HTMLSpanElement>("libraryPath"),
  chooseFolderBtn: $<HTMLButtonElement>("chooseFolderBtn"),
  resetFolderBtn: $<HTMLButtonElement>("resetFolderBtn"),
};

const CAPTION_MAX = 2000;
const LYRICS_MAX = 3500;

/** Bracketed text the API would sing rather than read as structure. */
interface StrayTag {
  text: string;
  line: number;
  suggestion: string | null;
}

let generating = false;
let timer: number | undefined;
let hasKey = false;
let takes: Take[] = [];
let currentDir: string | null = null;

const audio = new Audio();
audio.preload = "metadata";

// ------------------------------------------------------------ appearance

type Appearance = "system" | "light" | "dark";
const APPEARANCE_KEY = "musicmaxxer.appearance";
const darkMedia = window.matchMedia("(prefers-color-scheme: dark)");

const SUN_PATH =
  '<circle cx="12" cy="12" r="4"/><path d="M12 2v2.4M12 19.6V22M4.2 4.2l1.7 1.7M18.1 18.1l1.7 1.7M2 12h2.4M19.6 12H22M4.2 19.8l1.7-1.7M18.1 5.9l1.7-1.7"/>';
const MOON_PATH = '<path d="M20.5 14.4A8.5 8.5 0 1 1 9.6 3.5a7 7 0 0 0 10.9 10.9z"/>';

function getAppearance(): Appearance {
  const v = localStorage.getItem(APPEARANCE_KEY);
  return v === "light" || v === "dark" ? v : "system";
}

function resolveTheme(pref: Appearance): "light" | "dark" {
  return pref === "system" ? (darkMedia.matches ? "dark" : "light") : pref;
}

// SPEC §6: "System (auto), Light, Dark" — three radios in Settings, plus a
// one-click toggle in the titlebar. Both write the same preference.
function applyAppearance(pref: Appearance) {
  localStorage.setItem(APPEARANCE_KEY, pref);
  if (pref === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = pref;

  // The icon shows the mode a click would switch *to*, not the one that's
  // currently on — a moon in light mode invites you toward dark, and vice
  // versa.
  const resolved = resolveTheme(pref);
  els.themeIcon.innerHTML = resolved === "dark" ? SUN_PATH : MOON_PATH;
  els.themeBtn.title = `Appearance: ${pref}${pref === "system" ? ` (currently ${resolved})` : ""}`;

  for (const r of document.querySelectorAll<HTMLInputElement>('input[name="appearance"]')) {
    r.checked = r.value === pref;
  }
}

// Clicking sets an explicit preference for the mode the icon is showing,
// overriding System if that was selected.
els.themeBtn.addEventListener("click", () => {
  applyAppearance(resolveTheme(getAppearance()) === "dark" ? "light" : "dark");
});

for (const r of document.querySelectorAll<HTMLInputElement>('input[name="appearance"]')) {
  r.addEventListener("change", () => {
    if (r.checked) applyAppearance(r.value as Appearance);
  });
}

// Only matters while System is selected — an explicit choice does not care
// what the OS does.
darkMedia.addEventListener("change", () => {
  if (getAppearance() === "system") applyAppearance("system");
});

// -------------------------------------------------- unsaved-changes tracking

/** What a "current song project" is judged against for §6's abandon warning. */
interface FormSnapshot {
  title: string;
  caption: string;
  lyrics: string;
  instrumental: boolean;
}

let baseline: FormSnapshot | null = null;
/** The take whose recipe is currently loaded, so re-clicking it while clean
 *  does not reload and clobber the last notice for no reason. */
let loadedTakeDir: string | null = null;

function snapshotForm(): FormSnapshot {
  return {
    title: els.title.value,
    caption: els.caption.value,
    lyrics: els.lyrics.value,
    instrumental: instrumental(),
  };
}

function markClean() {
  baseline = snapshotForm();
}

function isDirty(): boolean {
  if (!baseline) return false;
  const cur = snapshotForm();
  return (
    cur.title !== baseline.title ||
    cur.caption !== baseline.caption ||
    cur.lyrics !== baseline.lyrics ||
    cur.instrumental !== baseline.instrumental
  );
}

// ------------------------------------------------------------------ notices

function notice(kind: "info" | "error" | "ok", html: string): HTMLDivElement {
  els.notices.innerHTML = "";
  const d = document.createElement("div");
  d.className = `notice ${kind === "info" ? "" : kind}`.trim();
  d.innerHTML = html;
  els.notices.appendChild(d);
  return d;
}

const clearNotices = () => (els.notices.innerHTML = "");

const escapeHtml = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

/// An in-app confirm, built from the same .scrim/.modal the Settings dialog
/// uses, rather than window.confirm(). Tauri's embedded webview has been
/// known to swallow native confirm/alert silently — no dialog, no error, the
/// call just resolves as cancelled — which is indistinguishable from a user
/// declining. `bodyHtml` is trusted; callers escape their own interpolation.
function confirmInApp(title: string, bodyHtml: string, confirmLabel: string): Promise<boolean> {
  return new Promise((resolve) => {
    const scrim = document.createElement("div");
    scrim.className = "scrim is-on";
    scrim.innerHTML = `
      <div class="modal" role="alertdialog" aria-modal="true">
        <div class="modal-head"><h2>${escapeHtml(title)}</h2></div>
        <div class="modal-body">${bodyHtml}</div>
        <div class="modal-foot">
          <div class="right">
            <button class="btn" id="confirmNo">Cancel</button>
            <button class="btn danger" id="confirmYes">${escapeHtml(confirmLabel)}</button>
          </div>
        </div>
      </div>`;
    document.body.appendChild(scrim);

    const finish = (ok: boolean) => {
      document.removeEventListener("keydown", onKey);
      scrim.remove();
      resolve(ok);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") finish(false);
    };
    document.addEventListener("keydown", onKey);
    scrim.addEventListener("click", (e) => {
      if (e.target === scrim) finish(false);
    });
    scrim.querySelector("#confirmNo")!.addEventListener("click", () => finish(false));
    scrim.querySelector("#confirmYes")!.addEventListener("click", () => finish(true));
  });
}

// -------------------------------------------------------------- the form

function autogrow(el: HTMLTextAreaElement, min = 0) {
  el.style.height = "auto";
  el.style.height = `${Math.max(el.scrollHeight, min)}px`;
}

function counts() {
  const cap = [...els.caption.value].length;
  const lyr = [...els.lyrics.value].length;

  els.capCount.textContent = `${cap.toLocaleString()} / ${CAPTION_MAX.toLocaleString()}`;
  els.lyrCount.textContent = `${lyr.toLocaleString()} / ${LYRICS_MAX.toLocaleString()}`;
  els.capCount.classList.toggle("over", cap > CAPTION_MAX);
  els.lyrCount.classList.toggle("over", lyr > LYRICS_MAX);

  refreshGenerate();
}

function instrumental(): boolean {
  return els.swInst.getAttribute("aria-pressed") === "true";
}

function refreshGenerate() {
  if (generating) return;
  const cap = [...els.caption.value].length;
  const lyr = [...els.lyrics.value].length;
  const tooLong = cap > CAPTION_MAX || lyr > LYRICS_MAX;
  const needsCaption = cap === 0;
  const needsLyrics = !instrumental() && els.lyrics.value.trim() === "";

  els.genBtn.disabled = !hasKey || tooLong || needsCaption || needsLyrics;
}

// SPEC §6.1: Instrumental keeps the field live and relabels it. Disabling it
// would cap the take at about three seconds (§3.5).
els.swInst.addEventListener("click", () => {
  const on = !instrumental();
  els.swInst.setAttribute("aria-pressed", String(on));
  els.lyricsLabel.textContent = on ? "Structure" : "Lyrics";
  els.lengthHint.textContent = on
    ? "sections set the length — words are ignored"
    : "sections set the length";
  els.lyrics.placeholder = on
    ? "[Intro]\n[Build Up]\n[Inst]\n[Solo]\n[Outro]"
    : "[Verse]\nOne line per line. Section tags above insert at the cursor.";
  refreshGenerate();
});

// SPEC §6.1: structure tags are the app's only control over take length
// (§3.5), so the bar stays visible whether or not the take has vocals. The
// list comes from Rust so there is one copy, not two that can drift.
async function buildTagBar() {
  const tags = await invoke<string[]>("structure_tags");
  els.tags.innerHTML = "";

  for (const tag of tags) {
    const b = document.createElement("button");
    b.className = "tag";
    b.type = "button";
    b.textContent = tag;
    b.addEventListener("click", () => {
      const ta = els.lyrics;
      const start = ta.selectionStart;
      const before = ta.value.slice(0, start);
      const insert = `${before && !before.endsWith("\n") ? "\n" : ""}${tag}\n`;
      ta.value = before + insert + ta.value.slice(ta.selectionEnd);
      const caret = start + insert.length;
      ta.focus();
      ta.setSelectionRange(caret, caret);
      ta.dispatchEvent(new Event("input"));
    });
    els.tags.appendChild(b);
  }
}

// --------------------------------------------------- stray-tag warning

let strays: StrayTag[] = [];
let lintTimer: number | undefined;

/// SPEC §3.1: only fourteen exact tags are recognised. Anything else in
/// brackets is sung aloud — including [Verse 1] — and the response gives no
/// hint that it happened, so it has to be caught before sending.
async function lintLyrics() {
  try {
    strays = await invoke<StrayTag[]>("lint_lyrics", { lyrics: els.lyrics.value });
  } catch {
    strays = [];
  }
  renderStrayWarning();
}

function renderStrayWarning() {
  if (!strays.length) {
    els.strayWarn.innerHTML = "";
    els.strayWarn.hidden = true;
    return;
  }

  const rows = strays
    .map(
      (s) =>
        `<li>line ${s.line}: <code>${escapeHtml(s.text)}</code>${
          s.suggestion ? ` — did you mean <code>${escapeHtml(s.suggestion)}</code>?` : ""
        }</li>`,
    )
    .join("");

  els.strayWarn.hidden = false;
  els.strayWarn.innerHTML = `
    <div class="notice warn">
      <strong>${strays.length} bracketed ${strays.length === 1 ? "line" : "lines"} will be sung.</strong>
      MiniMax recognises only the fourteen tags above — anything else in brackets
      becomes lyrics. Performance direction belongs in the Caption.
      <ul>${rows}</ul>
      ${
        strays.some((s) => s.suggestion)
          ? '<div class="row"><button class="btn" id="fixStrays">Fix the ones with suggestions</button></div>'
          : ""
      }
    </div>`;

  document.getElementById("fixStrays")?.addEventListener("click", fixStrays);
}

/// Replace each guessable stray with the tag it resembles. What was inside the
/// brackets is dropped, not moved — it is direction, and only the writer knows
/// how it should read in the Caption.
function fixStrays() {
  const lines = els.lyrics.value.split("\n");
  for (const s of strays) {
    if (!s.suggestion) continue;
    const index = s.line - 1;
    const line = lines[index];
    if (line !== undefined) lines[index] = line.split(s.text).join(s.suggestion);
  }
  els.lyrics.value = lines.join("\n");
  els.lyrics.dispatchEvent(new Event("input"));
  notice(
    "info",
    "Tags corrected. The wording inside them was dropped — if it was vocal or performance direction, add it to the Caption.",
  );
}

els.caption.addEventListener("input", () => {
  autogrow(els.caption);
  counts();
});
els.lyrics.addEventListener("input", () => {
  autogrow(els.lyrics, 208);
  counts();
  // Debounced: the check is local and cheap, but not worth a round trip per
  // keystroke while someone is mid-word.
  if (lintTimer !== undefined) window.clearTimeout(lintTimer);
  lintTimer = window.setTimeout(() => void lintLyrics(), 250);
});

// --------------------------------------------------------- output settings

// SPEC §6.3: model/format settings stay inline on Compose in a collapsed
// panel whose summary shows their current state, not the Settings dialog.
els.outputToggle.addEventListener("click", () => {
  const open = els.outputPanel.classList.toggle("is-on");
  els.outputToggle.setAttribute("aria-expanded", String(open));
});

// SPEC §3.1's six model IDs, minus the two cover models Compose can never
// use (no reference audio here) -- the list comes from Rust, with the RPM
// figure stashed on each option, so the summary line never drifts from the
// dropdown that produced it.
async function buildModelSelect() {
  const models = await invoke<ModelInfo[]>("available_models");
  els.modelSelect.innerHTML = "";
  for (const m of models) {
    const opt = document.createElement("option");
    opt.value = m.id;
    opt.textContent = `${m.id} — ${m.rpm} rpm${m.free_tier ? "" : " (paid)"}`;
    opt.dataset.rpm = String(m.rpm);
    els.modelSelect.appendChild(opt);
  }
}

function refreshOutputSummary() {
  const rpm = els.modelSelect.selectedOptions[0]?.dataset.rpm ?? "?";
  const khz = (Number(els.sampleRateSelect.value) / 1000).toFixed(1);
  els.outputToggle.textContent = `${els.modelSelect.value} · ${els.formatSelect.value} ${khz} kHz · ${rpm} rpm`;
}

function applySettingsPayload(payload: SettingsPayload) {
  els.modelSelect.value = payload.model;
  els.formatSelect.value = payload.format;
  els.sampleRateSelect.value = String(payload.sample_rate);
  els.bitrateSelect.disabled = payload.format !== "mp3";
  if (payload.bitrate !== null) els.bitrateSelect.value = String(payload.bitrate);

  els.libraryPath.textContent = payload.library_root;
  els.libraryPath.title = payload.library_root;
  els.resetFolderBtn.hidden = !payload.is_custom_library_root;

  refreshOutputSummary();
}

/** Whatever the panel currently shows for the library folder -- null means
 *  "using the platform default", matching what save_settings expects. */
function currentLibraryRootOverride(): string | null {
  return els.resetFolderBtn.hidden ? null : els.libraryPath.textContent;
}

/// Every control here persists the moment it changes, the same as ratings,
/// appearance and the API key -- there is no separate "Save settings" step.
async function saveOutputSettings(libraryRoot: string | null) {
  try {
    const payload = await invoke<SettingsPayload>("save_settings", {
      settings: {
        model: els.modelSelect.value,
        format: els.formatSelect.value,
        sample_rate: Number(els.sampleRateSelect.value),
        bitrate: els.formatSelect.value === "mp3" ? Number(els.bitrateSelect.value) : null,
        library_root: libraryRoot,
      },
    });
    applySettingsPayload(payload);
  } catch (e) {
    notice("error", escapeHtml(String(e)));
  }
}

els.modelSelect.addEventListener("change", () => {
  void saveOutputSettings(currentLibraryRootOverride());
});
els.formatSelect.addEventListener("change", () => {
  // Snappy: disable bitrate immediately rather than waiting on the round trip.
  els.bitrateSelect.disabled = els.formatSelect.value !== "mp3";
  void saveOutputSettings(currentLibraryRootOverride());
});
els.sampleRateSelect.addEventListener("change", () => {
  void saveOutputSettings(currentLibraryRootOverride());
});
els.bitrateSelect.addEventListener("change", () => {
  void saveOutputSettings(currentLibraryRootOverride());
});

els.chooseFolderBtn.addEventListener("click", async () => {
  const picked = await invoke<string | null>("pick_library_folder");
  if (!picked) return; // cancelled -- not an error
  await saveOutputSettings(picked);
  await refreshHistory();
});
els.resetFolderBtn.addEventListener("click", async () => {
  await saveOutputSettings(null);
  await refreshHistory();
});

// ------------------------------------------------------------------- keys

async function refreshKeyStatus() {
  const status = (await invoke<KeyStatus>("key_status")) ?? { minimax: false };
  hasKey = status.minimax;
  els.keyPill.dataset.state = hasKey ? "set" : "unset";
  els.keyPillText.textContent = hasKey ? "Key in Keychain" : "No API key";
  refreshGenerate();
}

// A saved key shows as bullets, the way every other password field does. The
// real key is never sent to the webview — this is a fixed-width placeholder
// standing in for "something is stored", not a masked value.
const MASK = "•".repeat(14);
let keyEdited = false;

function openSettings(prompt?: string) {
  els.keyWarn.innerHTML = prompt ? `<div class="notice">${escapeHtml(prompt)}</div>` : "";
  keyEdited = false;
  els.keyInput.value = hasKey ? MASK : "";
  els.keyInput.placeholder = "paste your MiniMax API key";
  applyAppearance(getAppearance()); // syncs the radios to the current choice
  els.scrim.classList.add("is-on");
  els.keyInput.focus();
  // Typing replaces the placeholder wholesale rather than appending to it.
  if (hasKey) els.keyInput.select();
}

els.keyInput.addEventListener("input", () => {
  keyEdited = true;
});

const closeSettings = () => els.scrim.classList.remove("is-on");

els.settingsBtn.addEventListener("click", () => openSettings());
els.modalCancel.addEventListener("click", closeSettings);
els.scrim.addEventListener("click", (e) => {
  if (e.target === els.scrim) closeSettings();
});

els.modalSave.addEventListener("click", async () => {
  // Untouched field: the stored key stands. Saving the bullets would overwrite
  // a working key with sixteen dots.
  if (!keyEdited) {
    closeSettings();
    return;
  }
  const key = els.keyInput.value.trim();
  if (!key) {
    closeSettings();
    return;
  }
  try {
    // A suspicious key still saves — only the API can judge it (SPEC §2).
    const warning = await invoke<string | null>("save_key", { key });
    els.keyInput.value = "";
    closeSettings();
    await refreshKeyStatus();
    if (warning) {
      notice("info", `<strong>Key saved.</strong> ${escapeHtml(warning)}.`);
    } else {
      clearNotices();
    }
  } catch (e) {
    els.keyWarn.innerHTML = `<div class="notice error">${escapeHtml(String(e))}</div>`;
  }
});

els.clearKey.addEventListener("click", async () => {
  await invoke("clear_key");
  els.keyInput.value = "";
  closeSettings();
  await refreshKeyStatus();
  notice("info", "Key cleared.");
});

// ----------------------------------------------------------- history rail

const mmss = (secs: number) =>
  `${Math.floor(secs / 60)}:${String(Math.floor(secs % 60)).padStart(2, "0")}`;

const clock = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? "—"
    : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
};

function visibleTakes(): Take[] {
  const q = els.histSearch.value.trim().toLowerCase();
  const min = Number(els.histRating.value);
  const exact = min === 5;

  return takes.filter((t) => {
    const matches = !q || `${t.title} ${t.caption}`.toLowerCase().includes(q);
    const rated = exact ? t.rating === 5 : t.rating >= min;
    return matches && rated;
  });
}

function renderHistory() {
  const list = els.histList;
  const shown = visibleTakes();
  els.histCount.textContent = String(takes.length);
  list.innerHTML = "";

  if (!shown.length) {
    const empty = document.createElement("div");
    empty.className = "hist-empty";
    empty.textContent = takes.length
      ? "No takes match that filter."
      : "No takes yet. Everything you generate is saved with its run.json.";
    list.appendChild(empty);
    return;
  }

  for (const t of shown) {
    const row = document.createElement("div");
    row.className = `hist-item${t.dir === currentDir ? " is-current" : ""}`;
    row.innerHTML = `
      <div class="hi-row1">
        <span>${clock(t.created_at)}</span><span>${escapeHtml(t.model)}</span>
        <span class="dur">${mmss(t.duration_secs)}</span>
      </div>
      <div class="hi-title">${escapeHtml(t.title)}
        <span style="font-family:var(--mono);font-size:10px;color:var(--text-3);font-weight:400">take ${t.take}</span>
      </div>
      <div class="hi-sub">${escapeHtml(t.caption)}</div>
      <div class="hi-foot">
        <div class="stars" role="group" aria-label="Rating"></div>
        <div class="hi-acts">
          <button class="mini" data-act="reveal">Reveal</button>
          <button class="mini danger" data-act="delete">Delete</button>
        </div>
      </div>`;

    const stars = row.querySelector(".stars")!;
    for (let n = 1; n <= 5; n++) {
      const b = document.createElement("button");
      b.className = `star${n <= t.rating ? " on" : ""}`;
      b.type = "button";
      b.textContent = n <= t.rating ? "★" : "☆";
      b.title = `${n} star${n === 1 ? "" : "s"}`;
      b.setAttribute("aria-label", b.title);
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        // Clicking the current rating again clears it.
        void rate(t, t.rating === n ? 0 : n);
      });
      stars.appendChild(b);
    }

    row.querySelector('[data-act="reveal"]')!.addEventListener("click", (e) => {
      e.stopPropagation();
      void invoke("reveal", { path: t.audio_path });
    });
    row.querySelector('[data-act="delete"]')!.addEventListener("click", (e) => {
      e.stopPropagation();
      void remove(t);
    });
    row.addEventListener("click", () => {
      play(t);
      void loadTakeIntoEditor(t);
    });

    list.appendChild(row);
  }
}

async function refreshHistory() {
  try {
    takes = await invoke<Take[]>("list_takes");
  } catch (e) {
    notice("error", escapeHtml(String(e)));
    takes = [];
  }
  renderHistory();
}

async function rate(take: Take, stars: number) {
  try {
    await invoke("set_rating", { dir: take.dir, stars });
    take.rating = stars;
    renderHistory();
  } catch (e) {
    notice("error", escapeHtml(String(e)));
  }
}

async function remove(take: Take) {
  const label = `${take.title} — take ${take.take}`;
  const ok = await confirmInApp(
    "Delete this take?",
    `<p>${escapeHtml(label)} — the audio and its run.json go with it.</p>`,
    "Delete",
  );
  if (!ok) return;

  try {
    await invoke("delete_take", { dir: take.dir });
    if (currentDir === take.dir) stopPlayback();
    await refreshHistory();
    notice("info", `Deleted ${escapeHtml(label)}.`);
  } catch (e) {
    notice("error", escapeHtml(String(e)));
  }
}

els.histSearch.addEventListener("input", renderHistory);
els.histRating.addEventListener("change", renderHistory);

// ---------------------------------------------------------------- player

function setPlayIcon(playing: boolean) {
  els.playIcon.innerHTML = playing
    ? '<path d="M7 5h4v14H7zM13 5h4v14h-4z" />'
    : '<path d="M8 5l12 7-12 7z" />';
  els.playBtn.setAttribute("aria-label", playing ? "Pause" : "Play");
}

/** Load a take into the player without starting it. */
function cue(take: Take) {
  currentDir = take.dir;
  // Streamed through the asset protocol rather than read into memory — a
  // four-minute wav is about 40 MB.
  audio.src = convertFileSrc(take.audio_path);
  els.playName.textContent = `${take.title} — take ${take.take}`;
  els.playDur.textContent = mmss(take.duration_secs);
  els.playPos.textContent = "0:00";
  els.seek.value = "0";
  paintSeek(0);
  els.player.classList.add("is-on");
  renderHistory();
}

function play(take: Take) {
  if (currentDir === take.dir) {
    void togglePlay();
    return;
  }
  cue(take);
  void audio.play().catch((e) => notice("error", escapeHtml(String(e))));
}

async function togglePlay() {
  if (audio.paused) await audio.play().catch(() => undefined);
  else audio.pause();
}

function stopPlayback() {
  audio.pause();
  audio.removeAttribute("src");
  audio.load();
  currentDir = null;
  els.player.classList.remove("is-on");
}

els.playBtn.addEventListener("click", () => void togglePlay());
els.revealBtn.addEventListener("click", () => {
  const take = takes.find((t) => t.dir === currentDir);
  if (take) void invoke("reveal", { path: take.audio_path });
});

audio.addEventListener("play", () => setPlayIcon(true));
audio.addEventListener("pause", () => setPlayIcon(false));
audio.addEventListener("ended", () => setPlayIcon(false));

audio.addEventListener("loadedmetadata", () => {
  if (Number.isFinite(audio.duration)) els.playDur.textContent = mmss(audio.duration);
});

function paintSeek(fraction: number) {
  els.seek.style.setProperty("--played", `${Math.max(0, Math.min(1, fraction)) * 100}%`);
}

audio.addEventListener("timeupdate", () => {
  if (!Number.isFinite(audio.duration) || audio.duration === 0) return;
  els.playPos.textContent = mmss(audio.currentTime);
  if (!seeking) {
    const fraction = audio.currentTime / audio.duration;
    els.seek.value = String(Math.round(fraction * 1000));
    paintSeek(fraction);
  }
});

let seeking = false;
els.seek.addEventListener("pointerdown", () => (seeking = true));
els.seek.addEventListener("input", () => {
  const fraction = Number(els.seek.value) / 1000;
  paintSeek(fraction);
  if (Number.isFinite(audio.duration)) {
    els.playPos.textContent = mmss(fraction * audio.duration);
  }
});
els.seek.addEventListener("change", () => {
  if (Number.isFinite(audio.duration)) {
    audio.currentTime = (Number(els.seek.value) / 1000) * audio.duration;
  }
  seeking = false;
});

/// Nudge playback. Shift for a bigger jump.
function scrub(seconds: number) {
  if (!currentDir || !Number.isFinite(audio.duration)) return;
  audio.currentTime = Math.max(0, Math.min(audio.duration, audio.currentTime + seconds));
}

// ------------------------------------------------------- recipes (song.md)

function applyRecipe(recipe: Recipe, source: string) {
  loadedTakeDir = null; // the caller sets this back if the source was a take
  els.title.value = recipe.title;
  els.caption.value = recipe.caption;
  els.lyrics.value = recipe.lyrics;

  const wantInstrumental = recipe.instrumental;
  if (wantInstrumental !== instrumental()) els.swInst.click();

  els.caption.dispatchEvent(new Event("input"));
  els.lyrics.dispatchEvent(new Event("input"));

  // SPEC §7.3: a missing reference never blocks the restore — everything else
  // comes back and the reference is flagged.
  if (recipe.cover) {
    notice(
      "info",
      `<strong>Restored from ${escapeHtml(source)}</strong> — title, caption, lyrics and flags.
       This is a cover recipe; its reference audio
       (<code>${escapeHtml(recipe.cover.reference_file)}</code>) is not loaded, because the
       Cover tab is not built yet.`,
    );
  } else {
    notice(
      "info",
      `<strong>Restored from ${escapeHtml(source)}</strong> — title, caption, lyrics and flags.`,
    );
  }
  markClean();
}

/// Load the recipe that produced a take back into the compose form, so
/// generating again starts from exactly what made it. Warns before discarding
/// anything typed since the last generate or load that hasn't been used yet.
async function loadTakeIntoEditor(t: Take) {
  if (loadedTakeDir === t.dir && !isDirty()) return;

  if (isDirty()) {
    const ok = await confirmInApp(
      "Abandon current song project?",
      "<p>The changes in the editor haven't been used to generate anything yet.</p>",
      "Discard and load",
    );
    if (!ok) return;
  }

  try {
    const recipe = await invoke<Recipe>("take_recipe", { dir: t.dir });
    applyRecipe(recipe, `${t.title} — take ${t.take}`);
    loadedTakeDir = t.dir;
  } catch (e) {
    notice("error", escapeHtml(String(e)));
  }
}

async function openRecipe(path: string) {
  try {
    const recipe = await invoke<Recipe>("read_recipe", { path });
    const name = path.split("/").pop() ?? path;
    applyRecipe(recipe, name);
  } catch (e) {
    notice("error", `<strong>${escapeHtml(String(e))}</strong>`);
  }
}

// Tauri's native drag-drop gives real filesystem paths, unlike the HTML5 API.
void getCurrentWebview().onDragDropEvent((event) => {
  const payload = event.payload;
  if (payload.type === "over" || payload.type === "enter") {
    document.body.classList.add("is-dropping");
    return;
  }
  document.body.classList.remove("is-dropping");
  if (payload.type !== "drop") return;

  const first = payload.paths[0];
  if (first) void openRecipe(first);
});

// -------------------------------------------------------------- generating

function startTimer() {
  let seconds = 0;
  els.elapsed.textContent = "00:00";
  timer = window.setInterval(() => {
    seconds += 1;
    const m = String(Math.floor(seconds / 60)).padStart(2, "0");
    const s = String(seconds % 60).padStart(2, "0");
    els.elapsed.textContent = `${m}:${s}`;
  }, 1000);
}

function stopTimer() {
  if (timer !== undefined) window.clearInterval(timer);
  timer = undefined;
}

function setGenerating(on: boolean) {
  generating = on;
  els.genPanel.classList.toggle("is-on", on);
  els.genBtn.textContent = on ? "Generating…" : "Generate";
  els.genBtn.disabled = on;
  if (on) startTimer();
  else {
    stopTimer();
    refreshGenerate();
  }
}

function showTake(result: TakeResult) {
  const mb = (result.bytes / (1024 * 1024)).toFixed(1);
  notice(
    "ok",
    `<strong>${escapeHtml(result.title)}</strong> — take ${result.take}, ${mmss(result.duration_secs)}
     <dl>
       <dt>duration</dt><dd>${result.duration_secs.toFixed(1)}s</dd>
       <dt>size</dt><dd>${mb} MB</dd>
       <dt>generated in</dt><dd>${result.call_secs.toFixed(1)}s</dd>
       <dt>trace</dt><dd>${escapeHtml(result.trace_id ?? "—")}</dd>
     </dl>
     <details><summary>Where is it?</summary>
       <code>${escapeHtml(result.dir)}</code>
     </details>`,
  );
}

function showFailure(f: Failure) {
  if (f.cancelled) {
    notice("info", "Cancelled. The request was dropped and nothing was written.");
    return;
  }
  const details = [
    f.code !== null ? `status_code ${f.code}` : null,
    f.traceId ? `trace_id ${f.traceId}` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  notice(
    "error",
    `<strong>${escapeHtml(f.message)}</strong>
     ${
       f.fields.length
         ? `<ul>${f.fields.map((i) => `<li>${escapeHtml(i.message)}</li>`).join("")}</ul>`
         : ""
     }
     ${details ? `<details><summary>Details</summary><code>${escapeHtml(details)}</code></details>` : ""}`,
  );
}

async function generate() {
  if (generating) return;

  // Last gate before three minutes and a rate-limit slot. A writer may want a
  // bracketed line sung, so this asks rather than blocks.
  if (strays.length) {
    const rows = strays
      .map((s) => `<li>line ${s.line}: <code>${escapeHtml(s.text)}</code></li>`)
      .join("");
    const ok = await confirmInApp(
      "Generate with unrecognised tags?",
      `<p>${strays.length} bracketed ${strays.length === 1 ? "line" : "lines"} will be sung aloud —
       only the fourteen listed tags are recognised as structure.</p><ul>${rows}</ul>`,
      "Generate anyway",
    );
    if (!ok) return;
  }

  clearNotices();
  setGenerating(true);

  try {
    const result = await invoke<TakeResult>("generate", {
      req: {
        title: els.title.value,
        caption: els.caption.value,
        lyrics: els.lyrics.value,
        instrumental: instrumental(),
        model: els.modelSelect.value,
        format: els.formatSelect.value,
        sample_rate: Number(els.sampleRateSelect.value),
        bitrate: els.formatSelect.value === "mp3" ? Number(els.bitrateSelect.value) : null,
      },
    });
    showTake(result);
    // What's on screen just produced this take, so it is the new "clean"
    // state — and re-clicking this row in a moment should not reload it.
    markClean();
    loadedTakeDir = result.dir;
    await refreshHistory();
    // Cue the new take without starting it — a four-minute autoplay after a
    // three-minute wait is startling, not helpful.
    const fresh = takes.find((t) => t.dir === result.dir);
    if (fresh) cue(fresh);
  } catch (e) {
    showFailure(toFailure(e));
  } finally {
    setGenerating(false);
  }
}

els.genBtn.addEventListener("click", generate);
els.cancelBtn.addEventListener("click", () => void invoke("cancel"));

const typing = () => {
  const el = document.activeElement;
  return el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement;
};

document.addEventListener("keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && !els.genBtn.disabled) {
    void generate();
    return;
  }
  if (e.key === "Escape" && els.scrim.classList.contains("is-on")) {
    closeSettings();
    return;
  }
  // Transport shortcuts, but never while the caret is in a text field.
  if (typing() || !currentDir) return;

  if (e.key === " ") {
    e.preventDefault();
    void togglePlay();
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    scrub(e.shiftKey ? -30 : -5);
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    scrub(e.shiftKey ? 30 : 5);
  }
});

// ------------------------------------------------------------------- boot

applyAppearance(getAppearance());
autogrow(els.caption);
autogrow(els.lyrics, 208);
counts();
markClean();
void buildTagBar();
void lintLyrics();
setPlayIcon(false);
paintSeek(0);
// The model select must exist before a settings payload can pick an option
// out of it, so this is sequential rather than a second fire-and-forget.
void buildModelSelect().then(async () => {
  try {
    applySettingsPayload(await invoke<SettingsPayload>("get_settings"));
  } catch (e) {
    notice("error", escapeHtml(String(e)));
  }
});
// First launch, or the key was cleared and never replaced: open Settings
// with the field already focused rather than leaving Generate silently
// disabled for someone who has no idea why.
void refreshKeyStatus().then(() => {
  if (!hasKey) openSettings("Enter your MiniMax API key to get started.");
});
// Cue the newest take on launch, so the transport is there rather than hidden
// until you happen to click a row.
void refreshHistory().then(() => {
  if (takes[0]) cue(takes[0]);
});
