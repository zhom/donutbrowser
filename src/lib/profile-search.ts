/**
 * The profile search grammar.
 *
 * One text box carries the whole filter, so the grammar has to survive whatever
 * is in it halfway through a keystroke: a bare word still means what it always
 * meant, and anything the parser does not recognise degrades to that bare-word
 * search instead of to an error or an empty table. `foo:bar` is free text
 * because `foo` is not a field, which is what keeps a pasted `https://x:8080`
 * or a `12:30` in a note searchable. Nothing here throws, and nothing here may
 * answer "no rows" because of syntax.
 *
 * Field names and values are ASCII slugs and are never translated, so a query
 * means the same thing in every locale; only the help panel's prose goes
 * through `t()`, keyed off the `labelKey` each field carries. The React layer
 * calls `parseProfileSearch` and `matchesProfile` and nothing else — every
 * lookup the matcher needs (a group's name for its id, which profiles are
 * running) arrives in the `ProfileSearchContext` the caller builds.
 *
 * Kept free of runtime imports so `profile-search.test.mjs` can load it
 * directly, the way `proxy-string.ts` is.
 */

import type { BrowserProfile } from "@/types";

export type ProfileSearchFieldKind =
  | "text"
  | "tags"
  | "id"
  | "lookup"
  | "enum"
  | "boolean"
  | "date"
  | "version";

export interface ProfileSearchField {
  /** Canonical token; the one the help panel teaches. */
  readonly key: string;
  readonly aliases: readonly string[];
  readonly kind: ProfileSearchFieldKind;
  /** Translation key describing the field to a human. */
  readonly labelKey: string;
  /** Accepted slugs, where the set is closed. Shown in the help panel. */
  readonly values?: readonly string[];
}

/**
 * The closed field vocabulary. Closed on purpose: a token only becomes a field
 * when it is in here, so adding a short everyday word (`ip`, `url`) would
 * silently turn someone's plain-text search into a filter.
 */
export const PROFILE_SEARCH_FIELDS: readonly ProfileSearchField[] = [
  { key: "name", aliases: [], kind: "text", labelKey: "search.fields.name" },
  {
    key: "tag",
    aliases: ["tags"],
    kind: "tags",
    labelKey: "search.fields.tag",
  },
  {
    key: "note",
    aliases: ["notes"],
    kind: "text",
    labelKey: "search.fields.note",
  },
  { key: "id", aliases: [], kind: "id", labelKey: "search.fields.id" },
  {
    key: "group",
    aliases: ["folder"],
    kind: "lookup",
    labelKey: "search.fields.group",
  },
  {
    key: "proxy",
    aliases: [],
    kind: "lookup",
    labelKey: "search.fields.proxy",
  },
  { key: "vpn", aliases: [], kind: "lookup", labelKey: "search.fields.vpn" },
  {
    key: "ext",
    aliases: ["extension"],
    kind: "lookup",
    labelKey: "search.fields.ext",
  },
  {
    key: "dns",
    aliases: [],
    kind: "enum",
    labelKey: "search.fields.dns",
    values: ["light", "normal", "pro", "pro_plus", "ultimate", "custom"],
  },
  {
    key: "os",
    aliases: [],
    kind: "enum",
    labelKey: "search.fields.os",
    values: ["macos", "windows", "linux"],
  },
  {
    key: "browser",
    aliases: [],
    kind: "text",
    labelKey: "search.fields.browser",
  },
  {
    key: "status",
    aliases: [],
    kind: "enum",
    labelKey: "search.fields.status",
    values: ["running", "stopped"],
  },
  {
    key: "sync",
    aliases: [],
    kind: "enum",
    labelKey: "search.fields.sync",
    values: ["disabled", "regular", "encrypted"],
  },
  {
    key: "email",
    aliases: ["owner"],
    kind: "text",
    labelKey: "search.fields.email",
  },
  {
    key: "version",
    aliases: [],
    kind: "version",
    labelKey: "search.fields.version",
  },
  {
    key: "locked",
    aliases: ["password"],
    kind: "boolean",
    labelKey: "search.fields.locked",
    values: ["yes", "no"],
  },
  {
    key: "ephemeral",
    aliases: [],
    kind: "boolean",
    labelKey: "search.fields.ephemeral",
    values: ["yes", "no"],
  },
  {
    key: "created",
    aliases: [],
    kind: "date",
    labelKey: "search.fields.created",
  },
  {
    key: "launched",
    aliases: ["lastlaunch"],
    kind: "date",
    labelKey: "search.fields.launched",
  },
];

/** Operator vocabulary, for the help panel. The token is the syntax itself. */
export const PROFILE_SEARCH_OPERATORS: readonly {
  readonly token: string;
  readonly labelKey: string;
}[] = [
  { token: "-tag:ads", labelKey: "search.operators.negate" },
  { token: 'group:"Client A"', labelKey: "search.operators.quote" },
  { token: "tag:ads or tag:seo", labelKey: "search.operators.or" },
  { token: "tag:ads,seo", labelKey: "search.operators.comma" },
  { token: "tag:=prod", labelKey: "search.operators.exact" },
  { token: "proxy:none", labelKey: "search.operators.none" },
  { token: "created:>=2026-01-01", labelKey: "search.operators.compare" },
];

/** Whole queries worth copying, for the help panel. */
export const PROFILE_SEARCH_EXAMPLES: readonly {
  readonly query: string;
  readonly labelKey: string;
}[] = [
  { query: 'status:running group:"Client A"', labelKey: "search.examples.a" },
  { query: "tag:none proxy:any", labelKey: "search.examples.b" },
  { query: "launched:>30d -tag:archived", labelKey: "search.examples.c" },
];

export type ProfileSearchOperator = "match" | "lt" | "lte" | "gt" | "gte";

interface FreeTextTerm {
  readonly type: "text";
  /** Already lowercased. */
  readonly value: string;
  readonly negated: boolean;
}

interface FieldTerm {
  readonly type: "field";
  readonly field: ProfileSearchField;
  readonly operator: ProfileSearchOperator;
  /** Alternatives from the comma shorthand; any one matching matches. */
  readonly values: readonly string[];
  readonly negated: boolean;
  /** `=value`: whole-value match rather than substring. */
  readonly exact: boolean;
  /** The value was quoted, so `none` and `any` are literal text. */
  readonly quoted: boolean;
}

export type ProfileSearchTerm = FreeTextTerm | FieldTerm;

export interface ParsedProfileSearch {
  /** AND across the groups, OR inside each one. */
  readonly groups: readonly (readonly ProfileSearchTerm[])[];
  /** Nothing left to filter on, so every profile matches. */
  readonly isEmpty: boolean;
}

export interface ProfileSearchContext {
  /** Group id to the name the table shows for it. Same for the three below. */
  readonly groupNames: ReadonlyMap<string, string>;
  readonly proxyNames: ReadonlyMap<string, string>;
  readonly vpnNames: ReadonlyMap<string, string>;
  readonly extensionGroupNames: ReadonlyMap<string, string>;
  readonly runningProfiles: ReadonlySet<string>;
  /** Epoch ms the relative dates count back from. Defaults to the wall clock. */
  readonly now?: number;
}

const FIELD_BY_TOKEN: ReadonlyMap<string, ProfileSearchField> = new Map(
  PROFILE_SEARCH_FIELDS.flatMap((field) =>
    [field.key, ...field.aliases].map(
      (token) => [token, field] as [string, ProfileSearchField],
    ),
  ),
);

const RESERVED_NONE = "none";
const RESERVED_ANY = "any";
const RESERVED_NEVER = "never";

const DAY_MS = 86_400_000;
const DURATION_UNITS: Readonly<Record<string, number>> = {
  h: 3_600_000,
  d: DAY_MS,
  w: 7 * DAY_MS,
  m: 30 * DAY_MS,
  y: 365 * DAY_MS,
};

interface QueryChar {
  readonly c: string;
  readonly quoted: boolean;
}

/**
 * Splits on whitespace outside double quotes. An unclosed quote runs to the end
 * of the input instead of being rejected: the query is re-parsed on every
 * keystroke, so `name:"unclosed` is a query being typed, not a mistake. Each
 * character remembers whether it was quoted, which is what keeps a separator
 * inside quotes (`group:"Acme, Inc"`) literal.
 */
function tokenize(raw: string): QueryChar[][] {
  const tokens: QueryChar[][] = [];
  let current: QueryChar[] = [];
  let quoted = false;
  for (const c of raw) {
    if (c === '"') {
      quoted = !quoted;
      continue;
    }
    if (!quoted && /\s/.test(c)) {
      if (current.length > 0) {
        tokens.push(current);
        current = [];
      }
      continue;
    }
    current.push({ c, quoted });
  }
  if (current.length > 0) tokens.push(current);
  return tokens;
}

function textOf(chars: readonly QueryChar[]): string {
  let out = "";
  for (const ch of chars) out += ch.c;
  return out;
}

function hasQuoted(chars: readonly QueryChar[]): boolean {
  return chars.some((ch) => ch.quoted);
}

/** Splits on an unquoted separator, dropping the empty pieces. */
function splitUnquoted(chars: readonly QueryChar[], sep: string): string[] {
  const parts: string[] = [];
  let current = "";
  for (const ch of chars) {
    if (ch.c === sep && !ch.quoted) {
      if (current.length > 0) parts.push(current);
      current = "";
      continue;
    }
    current += ch.c;
  }
  if (current.length > 0) parts.push(current);
  return parts;
}

interface SeparatorToken {
  readonly token: string;
  readonly operator: ProfileSearchOperator;
  readonly negates: boolean;
}

/** Longest first, so `>=` is never read as `>` followed by a stray `=`. */
const SEPARATORS: readonly SeparatorToken[] = [
  { token: ">=", operator: "gte", negates: false },
  { token: "<=", operator: "lte", negates: false },
  { token: "!=", operator: "match", negates: true },
  { token: ":", operator: "match", negates: false },
  { token: ">", operator: "gt", negates: false },
  { token: "<", operator: "lt", negates: false },
];

function separatorAt(
  chars: readonly QueryChar[],
  index: number,
): SeparatorToken | null {
  for (const candidate of SEPARATORS) {
    let hit = true;
    for (let i = 0; i < candidate.token.length; i++) {
      const ch = chars[index + i];
      if (!ch || ch.quoted || ch.c !== candidate.token[i]) {
        hit = false;
        break;
      }
    }
    if (hit) return candidate;
  }
  return null;
}

/** Comparisons only mean something where the values are ordered. */
function acceptsComparison(field: ProfileSearchField): boolean {
  return field.kind === "date" || field.kind === "version";
}

function freeText(value: string, negated: boolean): FreeTextTerm | null {
  const trimmed = value.trim();
  if (trimmed.length === 0) return null;
  return { type: "text", value: trimmed.toLowerCase(), negated };
}

function buildTerm(chars: readonly QueryChar[]): ProfileSearchTerm | null {
  let negated = false;
  let body = chars;
  const first = body[0];
  if (
    body.length > 1 &&
    first &&
    !first.quoted &&
    (first.c === "-" || first.c === "!")
  ) {
    negated = true;
    body = body.slice(1);
  }

  let found: { at: number; token: SeparatorToken } | null = null;
  for (let i = 0; i < body.length && !found; i++) {
    if (body[i].quoted) continue;
    const token = separatorAt(body, i);
    if (token) found = { at: i, token };
  }
  if (!found || found.at === 0) return freeText(textOf(body), negated);

  const name = textOf(body.slice(0, found.at)).toLowerCase();
  const field = FIELD_BY_TOKEN.get(name);
  // An unrecognised name is never an error: it is somebody's note holding a
  // URL, and returning zero rows for it would be the worst possible answer.
  if (!field) return freeText(textOf(body), negated);

  let operator = found.token.operator;
  let value = body.slice(found.at + found.token.token.length);
  // `created:>=2026-01-01` writes the comparison after the colon; `created>=...`
  // writes it instead of one. Both reach the same term.
  if (found.token.token === ":") {
    const inner = separatorAt(value, 0);
    if (inner && inner.operator !== "match") {
      operator = inner.operator;
      value = value.slice(inner.token.length);
    }
  }
  if (operator !== "match" && !acceptsComparison(field)) {
    return freeText(textOf(body), negated);
  }
  if (value.length === 0) return null;
  if (found.token.negates) negated = !negated;

  let exact = false;
  const lead = value[0];
  if (lead && !lead.quoted && lead.c === "=") {
    exact = true;
    value = value.slice(1);
    if (value.length === 0) return null;
  }

  const quoted = hasQuoted(value);
  const values = (quoted ? [textOf(value)] : splitUnquoted(value, ",")).map(
    (v) => v.toLowerCase(),
  );
  if (values.length === 0) return null;

  if (field.kind === "date") {
    const usable = values.filter((v) => parseDateValue(v) !== null);
    // A date that does not parse drops its own term and leaves the rest of the
    // query running, rather than filtering everything away.
    if (usable.length === 0) return null;
    return {
      type: "field",
      field,
      operator,
      values: usable,
      negated,
      exact,
      quoted,
    };
  }

  return {
    type: "field",
    field,
    operator,
    values,
    negated,
    exact,
    quoted,
  };
}

/**
 * Turns raw input into AND-ed groups of OR-ed terms. Total: every input, valid
 * or not, produces a result, and an input with nothing usable in it produces an
 * empty one that matches every profile.
 */
export function parseProfileSearch(raw: string): ParsedProfileSearch {
  const groups: ProfileSearchTerm[][] = [];
  let pendingOr = false;

  for (const chars of tokenize(raw)) {
    if (!hasQuoted(chars) && textOf(chars).toLowerCase() === "or") {
      // A dangling `or` at either end simply has nothing to join.
      pendingOr = groups.length > 0;
      continue;
    }
    const term = buildTerm(chars);
    if (!term) continue;
    const last = groups[groups.length - 1];
    if (pendingOr && last) {
      last.push(term);
    } else {
      groups.push([term]);
    }
    pendingOr = false;
  }

  return { groups, isEmpty: groups.length === 0 };
}

type DateValue =
  | { readonly kind: "relative"; readonly durationMs: number }
  | {
      readonly kind: "absolute";
      readonly startMs: number;
      readonly endMs: number;
    }
  | { readonly kind: "never" }
  | { readonly kind: "any" };

const RELATIVE_PATTERN = /^(\d+)([hdwmy])$/;
const ABSOLUTE_PATTERN = /^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?$/;

/** `null` for anything that is not a date, which is how a term gets dropped. */
function parseDateValue(value: string): DateValue | null {
  if (value === RESERVED_NEVER || value === RESERVED_NONE) {
    return { kind: "never" };
  }
  if (value === RESERVED_ANY) return { kind: "any" };

  const relative = RELATIVE_PATTERN.exec(value);
  if (relative) {
    const amount = Number.parseInt(relative[1], 10);
    const unit = DURATION_UNITS[relative[2]];
    if (unit === undefined) return null;
    return { kind: "relative", durationMs: amount * unit };
  }

  const absolute = ABSOLUTE_PATTERN.exec(value);
  if (!absolute) return null;
  const year = Number.parseInt(absolute[1], 10);
  if (absolute[2] === undefined) {
    return {
      kind: "absolute",
      startMs: new Date(year, 0, 1).getTime(),
      endMs: new Date(year + 1, 0, 1).getTime(),
    };
  }
  const month = Number.parseInt(absolute[2], 10);
  if (month < 1 || month > 12) return null;
  if (absolute[3] === undefined) {
    return {
      kind: "absolute",
      startMs: new Date(year, month - 1, 1).getTime(),
      endMs: new Date(year, month, 1).getTime(),
    };
  }
  const day = Number.parseInt(absolute[3], 10);
  const start = new Date(year, month - 1, day);
  // February 31st parses as March 3rd unless the roll-over is caught here.
  if (start.getMonth() !== month - 1 || start.getDate() !== day) return null;
  return {
    kind: "absolute",
    startMs: start.getTime(),
    endMs: new Date(year, month - 1, day + 1).getTime(),
  };
}

function matchesDate(
  seconds: number | undefined,
  operator: ProfileSearchOperator,
  value: string,
  now: number,
): boolean {
  const parsed = parseDateValue(value);
  if (!parsed) return false;
  if (parsed.kind === "never") return !seconds;
  if (parsed.kind === "any") return Boolean(seconds);
  if (!seconds) return false;
  const ts = seconds * 1000;

  if (parsed.kind === "relative") {
    // Read the way the question is asked, not the way the timestamps compare:
    // `launched:<7d` is "inside the last 7 days" and `launched:>30d` is "not
    // launched for over 30 days", which is the query an operator hunting cold
    // profiles actually wants.
    const threshold = now - parsed.durationMs;
    switch (operator) {
      case "gt":
        return ts < threshold;
      case "gte":
        return ts <= threshold;
      default:
        return ts >= threshold;
    }
  }

  switch (operator) {
    case "lt":
      return ts < parsed.startMs;
    case "lte":
      return ts < parsed.endMs;
    case "gt":
      return ts >= parsed.endMs;
    case "gte":
      return ts >= parsed.startMs;
    default:
      return ts >= parsed.startMs && ts < parsed.endMs;
  }
}

function compareVersions(a: string, b: string): number {
  const left = a.split(".");
  const right = b.split(".");
  const length = Math.max(left.length, right.length);
  for (let i = 0; i < length; i++) {
    const l = Number.parseInt(left[i] ?? "0", 10);
    const r = Number.parseInt(right[i] ?? "0", 10);
    const ln = Number.isNaN(l) ? 0 : l;
    const rn = Number.isNaN(r) ? 0 : r;
    if (ln !== rn) return ln < rn ? -1 : 1;
  }
  return 0;
}

function parseBoolean(value: string): boolean | null {
  if (value === "yes" || value === "true" || value === "1") return true;
  if (value === "no" || value === "false" || value === "0") return false;
  return null;
}

interface FieldValue {
  /** The profile has something here, even if its name cannot be resolved. */
  readonly present: boolean;
  /** Lowercased text to compare against. */
  readonly candidates: readonly string[];
}

function lookupValue(
  id: string | undefined,
  names: ReadonlyMap<string, string>,
): FieldValue {
  if (!id) return { present: false, candidates: [] };
  const name = names.get(id);
  return {
    present: true,
    candidates: name ? [name.toLowerCase()] : [],
  };
}

function fieldValue(
  profile: BrowserProfile,
  field: ProfileSearchField,
  ctx: ProfileSearchContext,
): FieldValue {
  const one = (value: string | undefined | null): FieldValue =>
    value
      ? { present: true, candidates: [value.toLowerCase()] }
      : { present: false, candidates: [] };

  switch (field.key) {
    case "name":
      return one(profile.name);
    case "note":
      return one(profile.note);
    case "browser":
      return one(profile.browser);
    case "version":
      return one(profile.version);
    case "email":
      return one(profile.created_by_email);
    case "id":
      return { present: true, candidates: [profile.id.toLowerCase()] };
    case "tag": {
      const tags = profile.tags ?? [];
      return {
        present: tags.length > 0,
        candidates: tags.map((tag) => tag.toLowerCase()),
      };
    }
    case "group":
      return lookupValue(profile.group_id, ctx.groupNames);
    case "proxy":
      return lookupValue(profile.proxy_id, ctx.proxyNames);
    case "vpn":
      return lookupValue(profile.vpn_id, ctx.vpnNames);
    case "ext":
      return lookupValue(profile.extension_group_id, ctx.extensionGroupNames);
    case "dns":
      return one(profile.dns_blocklist);
    case "os":
      return one(profile.host_os ?? profile.wayfern_config?.os);
    case "sync":
      return one(profile.sync_mode ?? "Disabled");
    case "status":
      return one(ctx.runningProfiles.has(profile.id) ? "running" : "stopped");
    default:
      return { present: false, candidates: [] };
  }
}

function matchesFieldValue(term: FieldTerm, value: string, actual: FieldValue) {
  if (!term.quoted && !term.exact) {
    if (value === RESERVED_NONE) return !actual.present;
    if (value === RESERVED_ANY) return actual.present;
  }
  if (term.field.kind === "id") {
    return actual.candidates.some((candidate) =>
      term.exact ? candidate === value : candidate.startsWith(value),
    );
  }
  if (term.field.kind === "enum") {
    // A prefix is enough, so `status:run` works while the user is still typing.
    return actual.candidates.some((candidate) =>
      term.exact ? candidate === value : candidate.startsWith(value),
    );
  }
  return actual.candidates.some((candidate) =>
    term.exact ? candidate === value : candidate.includes(value),
  );
}

function matchesFieldTerm(
  profile: BrowserProfile,
  term: FieldTerm,
  ctx: ProfileSearchContext,
): boolean {
  const field = term.field;

  if (field.kind === "boolean") {
    const actual =
      field.key === "locked"
        ? profile.password_protected === true
        : profile.ephemeral === true;
    return term.values.some((value) => parseBoolean(value) === actual);
  }

  if (field.kind === "date") {
    const now = ctx.now ?? Date.now();
    const seconds =
      field.key === "created" ? profile.created_at : profile.last_launch;
    return term.values.some((value) =>
      matchesDate(seconds, term.operator, value, now),
    );
  }

  if (field.kind === "version" && term.operator !== "match") {
    return term.values.some((value) => {
      const order = compareVersions(profile.version, value);
      switch (term.operator) {
        case "lt":
          return order < 0;
        case "lte":
          return order <= 0;
        case "gt":
          return order > 0;
        default:
          return order >= 0;
      }
    });
  }

  const actual = fieldValue(profile, field, ctx);
  return term.values.some((value) => matchesFieldValue(term, value, actual));
}

/**
 * What a bare word searches: the same three fields the box has always covered,
 * plus the id, so the trimmed id the table shows can be pasted straight back in.
 */
function matchesFreeText(profile: BrowserProfile, value: string): boolean {
  if (profile.name.toLowerCase().includes(value)) return true;
  if (profile.note?.toLowerCase().includes(value)) return true;
  if (profile.tags?.some((tag) => tag.toLowerCase().includes(value))) {
    return true;
  }
  return profile.id.toLowerCase().startsWith(value);
}

export function matchesProfile(
  profile: BrowserProfile,
  parsed: ParsedProfileSearch,
  ctx: ProfileSearchContext,
): boolean {
  for (const group of parsed.groups) {
    const hit = group.some((term) => {
      const matched =
        term.type === "text"
          ? matchesFreeText(profile, term.value)
          : matchesFieldTerm(profile, term, ctx);
      return term.negated ? !matched : matched;
    });
    if (!hit) return false;
  }
  return true;
}
