export type RequestLanguage = "curl" | "javascript" | "python";

export type SnippetLanguage = RequestLanguage | "json";

export const REQUEST_LANGUAGES: readonly RequestLanguage[] = [
  "curl",
  "javascript",
  "python",
];

export const PROFILE_ID_PLACEHOLDER = "PROFILE_ID";

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface RequestSpec {
  method: "GET" | "POST" | "PUT" | "DELETE";
  /** Joined onto the base URL as is, so it may be empty. */
  path: string;
  body?: { [key: string]: JsonValue };
  /** False for the few routes that answer without the bearer token. */
  auth?: boolean;
  /** False when the route answers with a status and no body. */
  returnsJson?: boolean;
}

export interface RequestTarget {
  baseUrl: string;
  token: string;
}

const INDENT = "  ";
const PY_INDENT = "    ";
const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;

function urlOf(spec: RequestSpec, target: RequestTarget): string {
  return `${target.baseUrl}${spec.path}`;
}

function shellQuote(text: string): string {
  return `'${text.replaceAll("'", `'\\''`)}'`;
}

export function curlSnippet(spec: RequestSpec, target: RequestTarget): string {
  const head =
    spec.method === "GET"
      ? `curl ${urlOf(spec, target)}`
      : `curl -X ${spec.method} ${urlOf(spec, target)}`;
  const flags: string[] = [];
  if (spec.auth !== false) {
    flags.push(`-H "Authorization: Bearer ${target.token}"`);
  }
  if (spec.body) {
    flags.push(`-H "Content-Type: application/json"`);
    flags.push(`-d ${shellQuote(JSON.stringify(spec.body))}`);
  }
  return [head, ...flags.map((flag) => `${INDENT}${flag}`)].join(" \\\n");
}

function jsLiteral(value: JsonValue, depth: number): string {
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value);
  }
  const pad = INDENT.repeat(depth + 1);
  const close = INDENT.repeat(depth);
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    return `[\n${value.map((item) => `${pad}${jsLiteral(item, depth + 1)},`).join("\n")}\n${close}]`;
  }
  const entries = Object.entries(value);
  if (entries.length === 0) return "{}";
  return `{\n${entries
    .map(
      ([key, item]) =>
        `${pad}${IDENTIFIER.test(key) ? key : JSON.stringify(key)}: ${jsLiteral(item, depth + 1)},`,
    )
    .join("\n")}\n${close}}`;
}

export function javascriptSnippet(
  spec: RequestSpec,
  target: RequestTarget,
): string {
  const options: string[] = [];
  if (spec.method !== "GET") options.push(`method: "${spec.method}",`);
  const headers: string[] = [];
  if (spec.auth !== false) {
    headers.push(`Authorization: "Bearer ${target.token}",`);
  }
  if (spec.body) headers.push(`"Content-Type": "application/json",`);
  if (headers.length > 0) {
    options.push(
      `headers: {\n${headers.map((line) => `${INDENT.repeat(2)}${line}`).join("\n")}\n${INDENT}},`,
    );
  }
  if (spec.body)
    options.push(`body: JSON.stringify(${jsLiteral(spec.body, 1)}),`);
  const url = JSON.stringify(urlOf(spec, target));
  const call =
    options.length === 0
      ? `const response = await fetch(${url});`
      : `const response = await fetch(${url}, {\n${options.map((line) => `${INDENT}${line}`).join("\n")}\n});`;
  const result =
    spec.returnsJson === false
      ? "console.log(response.status);"
      : "console.log(await response.json());";
  return `${call}\n${result}`;
}

export function pythonLiteral(value: JsonValue, depth = 0): string {
  if (value === null) return "None";
  if (value === true) return "True";
  if (value === false) return "False";
  if (typeof value !== "object") return JSON.stringify(value);
  const pad = PY_INDENT.repeat(depth + 1);
  const close = PY_INDENT.repeat(depth);
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    return `[\n${value.map((item) => `${pad}${pythonLiteral(item, depth + 1)},`).join("\n")}\n${close}]`;
  }
  const entries = Object.entries(value);
  if (entries.length === 0) return "{}";
  return `{\n${entries
    .map(
      ([key, item]) =>
        `${pad}${JSON.stringify(key)}: ${pythonLiteral(item, depth + 1)},`,
    )
    .join("\n")}\n${close}}`;
}

export function pythonSnippet(
  spec: RequestSpec,
  target: RequestTarget,
): string {
  const args = [JSON.stringify(urlOf(spec, target))];
  if (spec.auth !== false) {
    args.push(`headers={"Authorization": "Bearer ${target.token}"}`);
  }
  if (spec.body) args.push(`json=${pythonLiteral(spec.body, 1)}`);
  const result =
    spec.returnsJson === false
      ? "print(response.status_code)"
      : "print(response.json())";
  return [
    "import requests",
    "",
    `response = requests.${spec.method.toLowerCase()}(`,
    ...args.map((arg) => `${PY_INDENT}${arg},`),
    ")",
    result,
  ].join("\n");
}

export function requestSnippet(
  language: RequestLanguage,
  spec: RequestSpec,
  target: RequestTarget,
): string {
  switch (language) {
    case "curl":
      return curlSnippet(spec, target);
    case "javascript":
      return javascriptSnippet(spec, target);
    case "python":
      return pythonSnippet(spec, target);
  }
}

/** A continued shell command on one line, for places with room for one. */
export function oneLine(command: string): string {
  return command.replace(/ \\\n\s*/g, " ");
}

/**
 * A token is shown by its ends only. The copy carries the real value, so the
 * visible form only has to say whose token it is.
 */
export function maskToken(token: string | null | undefined): string {
  if (!token) return "TOKEN";
  if (token.length <= 12) return "•".repeat(8);
  return `${token.slice(0, 4)}…${token.slice(-4)}`;
}

export type LocalApiExampleId =
  | "listProfiles"
  | "runProfile"
  | "openUrl"
  | "killProfile"
  | "createProxy"
  | "openApi";

export interface ApiExample<Id extends string = string> {
  id: Id;
  request: RequestSpec;
  /** Needs a paid plan with browser automation; the route answers 402 otherwise. */
  paid?: boolean;
  response: string;
}

export const LOCAL_API_EXAMPLES: readonly ApiExample<LocalApiExampleId>[] = [
  {
    id: "listProfiles",
    request: { method: "GET", path: "/v1/profiles" },
    response: JSON.stringify(
      {
        profiles: [
          {
            id: "6f1c…",
            name: "Shopping",
            browser: "wayfern",
            proxy_id: null,
            tags: ["eu"],
            is_running: false,
          },
        ],
        total: 1,
      },
      null,
      2,
    ),
  },
  {
    id: "runProfile",
    paid: true,
    request: {
      method: "POST",
      path: `/v1/profiles/${PROFILE_ID_PLACEHOLDER}/run`,
      body: { url: "https://example.com", headless: false },
    },
    response: JSON.stringify(
      {
        profile_id: "6f1c…",
        remote_debugging_port: 9222,
        headless: false,
      },
      null,
      2,
    ),
  },
  {
    id: "openUrl",
    paid: true,
    request: {
      method: "POST",
      path: `/v1/profiles/${PROFILE_ID_PLACEHOLDER}/open-url`,
      body: { url: "https://example.com/account" },
      returnsJson: false,
    },
    response: "200 OK",
  },
  {
    id: "killProfile",
    paid: true,
    request: {
      method: "POST",
      path: `/v1/profiles/${PROFILE_ID_PLACEHOLDER}/kill`,
      returnsJson: false,
    },
    response: "204 No Content",
  },
  {
    id: "createProxy",
    request: {
      method: "POST",
      path: "/v1/proxies",
      body: {
        name: "Residential EU",
        proxy_settings: {
          proxy_type: "http",
          host: "proxy.example.com",
          port: 8080,
          username: "user",
          password: "secret",
        },
      },
    },
    response: JSON.stringify(
      {
        id: "a29d…",
        name: "Residential EU",
        proxy_settings: {
          proxy_type: "http",
          host: "proxy.example.com",
          port: 8080,
          username: "user",
          password: "secret",
        },
      },
      null,
      2,
    ),
  },
  {
    id: "openApi",
    request: { method: "GET", path: "/openapi.json", auth: false },
    response: JSON.stringify(
      {
        openapi: "3.1.0",
        info: { title: "donutbrowser", version: "…" },
        paths: { "/v1/profiles": {} },
      },
      null,
      2,
    ),
  },
];

export type RemoteMcpExampleId = "listTools" | "callTool" | "clientConfig";

export const MCP_SERVER_NAME = "donut-browser";

export const REMOTE_MCP_EXAMPLES: readonly ApiExample<
  Exclude<RemoteMcpExampleId, "clientConfig">
>[] = [
  {
    id: "listTools",
    request: {
      method: "POST",
      path: "",
      body: { jsonrpc: "2.0", id: 1, method: "tools/list" },
    },
    response: JSON.stringify(
      {
        jsonrpc: "2.0",
        id: 1,
        result: {
          tools: [
            {
              name: "list_profiles",
              description: "List all Wayfern browser profiles",
            },
          ],
        },
      },
      null,
      2,
    ),
  },
  {
    id: "callTool",
    request: {
      method: "POST",
      path: "",
      body: {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "list_profiles", arguments: {} },
      },
    },
    response: JSON.stringify(
      {
        jsonrpc: "2.0",
        id: 2,
        result: { content: [{ type: "text", text: "[ … ]" }] },
      },
      null,
      2,
    ),
  },
];

/** The entry most MCP clients accept in their `mcpServers` map. */
export function mcpClientConfig(url: string, token: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        [MCP_SERVER_NAME]: {
          type: "http",
          url,
          headers: { Authorization: `Bearer ${token}` },
        },
      },
    },
    null,
    2,
  );
}
