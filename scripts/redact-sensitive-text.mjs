import { Buffer } from "node:buffer";
import process from "node:process";
import { pathToFileURL } from "node:url";

const URL_PATTERN = /\b[a-z][a-z\d+.-]{1,20}:\/\/[^\s<>"'`]+/giu;
const PRIVATE_KEY_PATTERN =
  /-----BEGIN [^-\r\n]*PRIVATE KEY-----[\s\S]*?-----END [^-\r\n]*PRIVATE KEY-----/giu;
const BEARER_PATTERN = /\bBearer\s+[A-Za-z\d._~+/=-]+/giu;
const SECRET_ASSIGNMENT_PATTERN =
  /\b(?:api[_-]?key|authorization|password|passwd|private[_-]?key|proxy[_-]?(?:password|username)|refresh[_-]?token|secret|token|username)\b\s*[:=]\s*[^\s,;]+/giu;
const JWT_PATTERN = /\beyJ[A-Za-z\d_-]+\.[A-Za-z\d_-]+\.[A-Za-z\d_-]+\b/gu;
const TOKEN_PATTERN =
  /\b(?:gh[oprsu]_[A-Za-z\d]{20,}|github_pat_[A-Za-z\d_]{20,}|sk-[A-Za-z\d_-]{20,}|xox[baprs]-[A-Za-z\d-]{20,})\b/gu;
const EMAIL_PATTERN = /\b[A-Z\d._%+-]+@[A-Z\d.-]+\.[A-Z]{2,}\b/giu;
const UNIX_HOME_PATTERN = /\/(?:Users|home)\/[^/\s]+/gu;
const WINDOWS_HOME_PATTERN = /\b[A-Z]:\\Users\\[^\\\s]+/giu;
const IPV4_PATTERN =
  /\b(?:25[0-5]|2[0-4]\d|1?\d?\d)(?:\.(?:25[0-5]|2[0-4]\d|1?\d?\d)){3}\b/gu;
const DOMAIN_PATTERN = /\b(?:[a-z\d-]+\.)+[a-z]{2,}\b/giu;
const UUID_PATTERN =
  /\b[\da-f]{8}-[\da-f]{4}-[1-8][\da-f]{3}-[89ab][\da-f]{3}-[\da-f]{12}\b/giu;

function safeUrlLabel(value) {
  try {
    const parsed = new URL(value);
    return `${parsed.protocol}//<redacted>`;
  } catch {
    return "<redacted-url>";
  }
}

export function sensitiveVariants(values) {
  const variants = new Set();
  for (const rawValue of values ?? []) {
    const value = String(rawValue ?? "").trim();
    if (value.length < 4) continue;
    variants.add(value);
    variants.add(encodeURIComponent(value));
    variants.add(Buffer.from(value).toString("base64"));
    try {
      const parsed = new URL(value);
      for (const component of [
        parsed.username,
        parsed.password,
        parsed.hostname,
        parsed.host,
      ]) {
        if (component.length >= 4) {
          variants.add(component);
          variants.add(decodeURIComponent(component));
          variants.add(encodeURIComponent(decodeURIComponent(component)));
        }
      }
    } catch {
      // Non-URL secrets are already covered by their literal and encoded forms.
    }
  }
  return [...variants].sort((left, right) => right.length - left.length);
}

export function redactSensitiveText(text, { sensitiveValues = [] } = {}) {
  let redacted = String(text ?? "");
  for (const value of sensitiveVariants(sensitiveValues)) {
    redacted = redacted.split(value).join("<redacted-secret>");
  }
  return redacted
    .replace(PRIVATE_KEY_PATTERN, "<redacted-private-key>")
    .replace(URL_PATTERN, safeUrlLabel)
    .replace(BEARER_PATTERN, "Bearer <redacted-secret>")
    .replace(SECRET_ASSIGNMENT_PATTERN, "<redacted-secret>")
    .replace(JWT_PATTERN, "<redacted-token>")
    .replace(TOKEN_PATTERN, "<redacted-token>")
    .replace(EMAIL_PATTERN, "<redacted-email>")
    .replace(UNIX_HOME_PATTERN, "/<redacted-home>")
    .replace(WINDOWS_HOME_PATTERN, "<redacted-home>")
    .replace(IPV4_PATTERN, "<redacted-ip>")
    .replace(DOMAIN_PATTERN, "<redacted-domain>")
    .replace(UUID_PATTERN, "<redacted-identifier>");
}

export function redactIssueBody(text) {
  const sections = String(text ?? "").split(/^###\s+/mu);
  const preamble = redactSensitiveText(sections.shift() ?? "").trim();
  const safeSections = sections.map((section) => {
    const newline = section.indexOf("\n");
    if (newline < 0) return redactSensitiveText(section);
    const heading = section.slice(0, newline).trim();
    const value = section.slice(newline + 1).trim();
    const safeValue = /^(?:error logs or screenshots|logs|screenshots)$/iu.test(
      heading,
    )
      ? "[omitted from automated processing]"
      : redactSensitiveText(value);
    return `${heading}\n${safeValue}`;
  });
  return [preamble, ...safeSections.map((section) => `### ${section}`)]
    .filter(Boolean)
    .join("\n\n");
}

async function runCli() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) input += chunk;
  process.stdout.write(
    process.argv.includes("--issue-body")
      ? redactIssueBody(input)
      : redactSensitiveText(input),
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  await runCli();
}
