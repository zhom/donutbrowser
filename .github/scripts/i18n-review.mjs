#!/usr/bin/env node
// Translation review for .github/workflows/i18n-review.yml.
//
//   node .github/scripts/i18n-review.mjs collect
//   node .github/scripts/i18n-review.mjs report
//
// `collect` finds the locale strings a push or pull request changed and
// writes a prompt that pairs each one with its English source. It also finds,
// with no model involved, strings whose English text changed while a
// translation stayed as it was.
//
// `report` merges those deterministic findings with Copilot's answer (when
// there is one) into the job summary, warning annotations on a push, and a
// single comment on a pull request that is edited in place on every run.
//
// src/lib/i18n-parity.test.mjs already fails CI on missing keys, empty
// strings and dropped placeholders. This covers what a key-set check cannot
// see: a wrong meaning, the wrong language, English left untranslated, and a
// translation left stale when its English source changed. It is advisory and
// never fails the build on its findings.
//
// Pull request files are fetched through the API and parsed as data. Nothing
// from the pull request is checked out or executed.

import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const LOCALES_DIR = "src/i18n/locales";
const OUT = process.env.I18N_REVIEW_DIR || "/tmp/i18n-review";
const API = process.env.GITHUB_API_URL || "https://api.github.com";
const MARKER = "<!-- donut-translation-review -->";
// The prompt travels as one 128 KiB argument next to the system prompt.
const MAX_PROMPT_BYTES = 80_000;
const MAX_FINDINGS = 60;

const env = (name) => process.env[name] ?? "";

async function api(pathname, { accept, method = "GET", body } = {}) {
  const res = await fetch(`${API}${pathname}`, {
    method,
    headers: {
      Authorization: `Bearer ${env("GH_TOKEN")}`,
      Accept: accept ?? "application/vnd.github+json",
      "User-Agent": "donut-translation-review",
      ...(body ? { "Content-Type": "application/json" } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  return res;
}

async function apiJson(pathname, options) {
  const res = await api(pathname, options);
  if (!res.ok) {
    throw new Error(`${options?.method ?? "GET"} ${pathname}: HTTP ${res.status}`);
  }
  return res.status === 204 ? null : res.json();
}

const enc = encodeURIComponent;

/** File names in the locales directory at `ref`, or null when unreadable. */
async function listLocales(repo, ref) {
  const res = await api(`/repos/${repo}/contents/${LOCALES_DIR}?ref=${enc(ref)}`);
  if (!res.ok) return null;
  const entries = await res.json();
  if (!Array.isArray(entries)) return null;
  return entries
    .filter((e) => e.type === "file" && e.name.endsWith(".json"))
    .map((e) => e.name);
}

async function readLocale(repo, ref, name) {
  const res = await api(
    `/repos/${repo}/contents/${LOCALES_DIR}/${enc(name)}?ref=${enc(ref)}`,
    { accept: "application/vnd.github.raw+json" },
  );
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`reading ${name} at ${ref}: HTTP ${res.status}`);
  return res.text();
}

function flatten(value, prefix = "", out = new Map()) {
  for (const [key, child] of Object.entries(value ?? {})) {
    const full = prefix ? `${prefix}.${key}` : key;
    if (child !== null && typeof child === "object" && !Array.isArray(child)) {
      flatten(child, full, out);
    } else if (typeof child === "string") {
      out.set(full, child);
    }
  }
  return out;
}

function parse(text) {
  if (text == null) return { flat: new Map(), ok: true };
  try {
    return { flat: flatten(JSON.parse(text)), ok: true };
  } catch {
    return { flat: new Map(), ok: false };
  }
}

/** 1-based line of `"leaf": "value"` in the file, or 0 when not found. */
function lineOf(text, key, value) {
  if (!text) return 0;
  const leaf = key.slice(key.lastIndexOf(".") + 1);
  const needle = `${JSON.stringify(leaf)}: ${JSON.stringify(value)}`;
  const at = text.indexOf(needle);
  return at < 0 ? 0 : text.slice(0, at).split("\n").length;
}

function languageName(code) {
  try {
    return new Intl.DisplayNames(["en"], { type: "language" }).of(code) ?? code;
  } catch {
    return code;
  }
}

function setOutput(name, value) {
  if (env("GITHUB_OUTPUT")) appendFileSync(env("GITHUB_OUTPUT"), `${name}=${value}\n`);
}

function skip(reason) {
  console.log(`Nothing to review: ${reason}`);
  writeFileSync(
    path.join(OUT, "state.json"),
    JSON.stringify({ skipped: reason, reviewed: [], stale: [], broken: [] }),
  );
  setOutput("review", "false");
}

async function resolveRefs() {
  const repo = env("GITHUB_REPOSITORY");
  if (env("EVENT_NAME") === "push") {
    const before = env("BEFORE_SHA");
    if (!/^[0-9a-f]{40}$/.test(before) || /^0+$/.test(before)) return null;
    return { baseRepo: repo, baseRef: before, headRepo: repo, headRef: env("HEAD_SHA") };
  }
  const baseSha = env("BASE_SHA");
  const headSha = env("HEAD_SHA");
  // Compare against the merge base, so commits that landed on main after the
  // branch point are not mistaken for changes in this pull request.
  let baseRef = baseSha;
  try {
    const cmp = await apiJson(`/repos/${repo}/compare/${baseSha}...${headSha}`);
    baseRef = cmp?.merge_base_commit?.sha || baseSha;
  } catch {
    // A head commit the base repository cannot resolve: keep the base tip.
  }
  return { baseRepo: repo, baseRef, headRepo: env("HEAD_REPO") || repo, headRef: headSha };
}

async function collect() {
  mkdirSync(OUT, { recursive: true });
  const refs = await resolveRefs();
  if (!refs) return skip("no earlier commit to compare with");

  let { headRepo } = refs;
  let headNames = await listLocales(headRepo, refs.headRef);
  if (!headNames && headRepo !== refs.baseRepo) {
    headRepo = refs.baseRepo;
    headNames = await listLocales(headRepo, refs.headRef);
  }
  const baseNames = await listLocales(refs.baseRepo, refs.baseRef);
  if (!headNames || !baseNames) return skip("the locale files could not be read");
  if (!headNames.includes("en.json")) return skip("en.json is missing");

  const locales = {};
  for (const name of headNames) {
    const code = name.replace(/\.json$/, "");
    const [baseText, headText] = await Promise.all([
      baseNames.includes(name) ? readLocale(refs.baseRepo, refs.baseRef, name) : null,
      readLocale(headRepo, refs.headRef, name),
    ]);
    locales[code] = { base: parse(baseText).flat, head: parse(headText), headText };
  }

  const en = locales.en;
  const broken = Object.entries(locales)
    .filter(([, l]) => !l.head.ok)
    .map(([code]) => code);

  // Deterministic: the English source changed, the translation did not.
  const stale = [];
  for (const [key, value] of en.head.flat) {
    const before = en.base.get(key);
    if (before === undefined || before === value) continue;
    for (const [code, l] of Object.entries(locales)) {
      if (code === "en" || !l.head.ok) continue;
      const now = l.head.flat.get(key);
      if (now !== undefined && now === l.base.get(key) && now !== value) {
        stale.push({ locale: code, key, line: lineOf(l.headText, key, now) });
      }
    }
  }

  // For the model: every translation this change added or edited.
  const changed = new Map();
  for (const [code, l] of Object.entries(locales)) {
    if (code === "en" || !l.head.ok) continue;
    for (const [key, value] of l.head.flat) {
      if (l.base.get(key) === value || !en.head.flat.has(key)) continue;
      if (!changed.has(key)) changed.set(key, new Map());
      changed.get(key).set(code, value);
    }
  }

  const reviewed = [];
  const reviewedLocales = new Set();
  const blocks = [];
  let bytes = 0;
  let notReviewed = 0;
  for (const key of [...changed.keys()].sort()) {
    const lines = [`key: ${key}`, `en: ${JSON.stringify(en.head.flat.get(key))}`];
    for (const [code, value] of [...changed.get(key)].sort()) {
      lines.push(`${code}: ${JSON.stringify(value)}`);
    }
    const block = `${lines.join("\n")}\n`;
    const size = Buffer.byteLength(block);
    if (bytes + size > MAX_PROMPT_BYTES) {
      notReviewed += 1;
      continue;
    }
    bytes += size;
    blocks.push(block);
    for (const [code, value] of changed.get(key)) {
      reviewedLocales.add(code);
      reviewed.push({ locale: code, key, line: lineOf(locales[code].headText, key, value) });
    }
  }

  const codes = [...reviewedLocales].sort();
  writeFileSync(
    path.join(OUT, "prompt-user.txt"),
    [
      `Locales: ${codes.map((c) => `${c} (${languageName(c)})`).join(", ")}`,
      "",
      "Changed strings. Each block is one key: the English source, then the new translations.",
      "",
      blocks.join("\n"),
    ].join("\n"),
  );
  writeFileSync(
    path.join(OUT, "state.json"),
    JSON.stringify({ reviewed, stale, broken, notReviewed, changedKeys: changed.size }),
  );
  console.log(
    `${changed.size} changed key(s), ${reviewed.length} translation(s) to review, ` +
      `${notReviewed} key(s) over the size limit, ${stale.length} stale translation(s)`,
  );
  setOutput("review", reviewed.length > 0 ? "true" : "false");
}

// --- report -----------------------------------------------------------------

const clip = (text, max) => {
  const flat = String(text ?? "").replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max - 3)}...` : flat;
};

// Model text goes into a public comment: no links, no HTML, no pings.
const safeProse = (text, max) =>
  clip(text, max)
    .replace(/[<>[\]]/g, (c) => `\\${c}`)
    .replace(/@/g, "@⁠");

const safeCode = (text, max) => clip(text, max).replace(/`/g, "'");

// Workflow-command escaping, as in @actions/core.
const escapeData = (s) =>
  String(s).replace(/%/g, "%25").replace(/\r/g, "%0D").replace(/\n/g, "%0A");
const escapeProperty = (s) => escapeData(s).replace(/:/g, "%3A").replace(/,/g, "%2C");

function readAiFindings(state) {
  if (env("AI_STATUS") !== "ok") return [];
  let answer;
  try {
    answer = JSON.parse(readFileSync(path.join(OUT, "ai.json"), "utf8"));
  } catch {
    return [];
  }
  const known = new Map(state.reviewed.map((r) => [`${r.locale}\u0000${r.key}`, r]));
  const out = [];
  for (const f of Array.isArray(answer?.findings) ? answer.findings : []) {
    const hit = known.get(`${f?.locale}\u0000${f?.key}`);
    if (!hit || !String(f?.problem ?? "").trim()) continue;
    out.push({ ...hit, problem: f.problem, suggestion: f.suggestion });
    if (out.length >= MAX_FINDINGS) break;
  }
  return out;
}

function buildReport(state, findings) {
  const aiStatus = env("AI_STATUS");
  const lines = [MARKER, "### Translation review", ""];
  if (state.skipped) {
    lines.push(`Nothing reviewed: ${state.skipped}.`);
    return { text: lines.join("\n"), hasFindings: false };
  }
  const locales = [...new Set(state.reviewed.map((r) => r.locale))].sort();
  const keyCount = new Set(state.reviewed.map((r) => r.key)).size;
  if (keyCount > 0) {
    lines.push(`Checked ${keyCount} changed key(s) in ${locales.join(", ")} against English.`, "");
  }

  if (state.broken.length > 0) {
    lines.push(`Not valid JSON: ${state.broken.map((c) => `\`${c}.json\``).join(", ")}.`, "");
  }

  if (findings.length > 0) {
    lines.push("Translations that need a look:", "");
    for (const f of findings) {
      let line = `- \`${f.locale}\` \`${safeCode(f.key, 120)}\`: ${safeProse(f.problem, 300)}`;
      if (String(f.suggestion ?? "").trim()) {
        line += ` Suggested: \`${safeCode(f.suggestion, 300)}\``;
      }
      lines.push(line);
    }
    lines.push("");
  }

  if (state.stale.length > 0) {
    const byKey = new Map();
    for (const s of state.stale) {
      if (!byKey.has(s.key)) byKey.set(s.key, []);
      byKey.get(s.key).push(s.locale);
    }
    lines.push("English changed, the translation did not:", "");
    for (const [key, codes] of [...byKey].slice(0, MAX_FINDINGS)) {
      lines.push(`- \`${key}\` in ${codes.sort().join(", ")}`);
    }
    if (byKey.size > MAX_FINDINGS) lines.push(`- and ${byKey.size - MAX_FINDINGS} more`);
    lines.push("");
  }

  if (state.notReviewed > 0) {
    lines.push(`${state.notReviewed} more changed key(s) were over the size limit and not reviewed.`, "");
  }
  if (keyCount > 0 && aiStatus !== "ok") {
    lines.push("The Copilot review did not run, so only the checks without a model apply.", "");
  } else if (keyCount > 0 && findings.length === 0) {
    lines.push("Copilot found no problems in the changed translations.", "");
  }
  lines.push("Advisory. Copilot can be wrong; the English source is what counts.");

  const hasFindings = findings.length + state.stale.length + state.broken.length > 0;
  return { text: lines.join("\n"), hasFindings };
}

async function upsertComment(body, hasFindings) {
  const repo = env("GITHUB_REPOSITORY");
  const number = env("PR_NUMBER");
  let existing = null;
  for (let page = 1; page <= 10 && !existing; page++) {
    const comments = await apiJson(
      `/repos/${repo}/issues/${number}/comments?per_page=100&page=${page}`,
    );
    existing = comments.find(
      (c) => c.user?.type === "Bot" && String(c.body ?? "").startsWith(MARKER),
    );
    if (comments.length < 100) break;
  }
  if (existing) {
    await apiJson(`/repos/${repo}/issues/comments/${existing.id}`, {
      method: "PATCH",
      body: { body },
    });
    console.log(`Updated comment ${existing.id}`);
  } else if (hasFindings) {
    const created = await apiJson(`/repos/${repo}/issues/${number}/comments`, {
      method: "POST",
      body: { body },
    });
    console.log(`Created comment ${created.id}`);
  } else {
    console.log("No findings and no earlier comment: not commenting");
  }
}

async function report() {
  const state = JSON.parse(readFileSync(path.join(OUT, "state.json"), "utf8"));
  const findings = readAiFindings(state);
  const { text, hasFindings } = buildReport(state, findings);

  if (env("GITHUB_STEP_SUMMARY")) appendFileSync(env("GITHUB_STEP_SUMMARY"), `${text}\n`);

  if (env("EVENT_NAME") === "push") {
    const annotate = (item, message) => {
      const props = [
        `file=${escapeProperty(`${LOCALES_DIR}/${item.locale}.json`)}`,
        ...(item.line ? [`line=${item.line}`] : []),
        `title=${escapeProperty("Translation review")}`,
      ];
      console.log(`::warning ${props.join(",")}::${escapeData(message)}`);
    };
    for (const f of findings) {
      annotate(f, `${f.locale} ${clip(f.key, 120)}: ${clip(f.problem, 300)}`);
    }
    for (const s of state.stale.slice(0, MAX_FINDINGS)) {
      annotate(s, `${s.locale} ${s.key}: the English text changed, this translation did not`);
    }
  } else if (env("PR_NUMBER")) {
    await upsertComment(text, hasFindings);
  }
}

const command = process.argv[2];
const run = { collect, report }[command];
if (!run) {
  console.error("usage: i18n-review.mjs collect|report");
  process.exit(2);
}
await run();
