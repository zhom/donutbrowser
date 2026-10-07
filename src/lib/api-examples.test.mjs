import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  curlSnippet,
  javascriptSnippet,
  LOCAL_API_EXAMPLES,
  maskToken,
  mcpClientConfig,
  oneLine,
  PROFILE_ID_PLACEHOLDER,
  pythonLiteral,
  pythonSnippet,
  REMOTE_MCP_EXAMPLES,
  REQUEST_LANGUAGES,
  requestSnippet,
} from "./api-examples.ts";

const target = { baseUrl: "http://127.0.0.1:10108", token: "tok" };
const AsyncFunction = (async () => {}).constructor;

test("curl puts the method, the bearer and the body on their own lines", () => {
  assert.equal(
    curlSnippet({ method: "GET", path: "/v1/profiles" }, target),
    'curl http://127.0.0.1:10108/v1/profiles \\\n  -H "Authorization: Bearer tok"',
  );
  assert.equal(
    curlSnippet(
      { method: "POST", path: "/v1/x", body: { note: "it's" } },
      target,
    ),
    [
      "curl -X POST http://127.0.0.1:10108/v1/x \\",
      '  -H "Authorization: Bearer tok" \\',
      '  -H "Content-Type: application/json" \\',
      `  -d '{"note":"it'\\''s"}'`,
    ].join("\n"),
  );
  assert.equal(
    curlSnippet({ method: "GET", path: "/openapi.json", auth: false }, target),
    "curl http://127.0.0.1:10108/openapi.json",
  );
});

test("the JavaScript form is valid code and keeps the body's values", () => {
  for (const example of [...LOCAL_API_EXAMPLES, ...REMOTE_MCP_EXAMPLES]) {
    const code = javascriptSnippet(example.request, target);
    assert.doesNotThrow(() => new AsyncFunction(code), example.id);
  }
  const code = javascriptSnippet(
    {
      method: "POST",
      path: "/v1/x",
      body: { plain: 1, "needs-quotes": [true, null] },
    },
    target,
  );
  assert.match(code, /\bplain: 1,/);
  assert.match(code, /"needs-quotes": \[/);
  assert.equal(
    javascriptSnippet({ method: "GET", path: "/o", auth: false }, target),
    'const response = await fetch("http://127.0.0.1:10108/o");\nconsole.log(await response.json());',
  );
  assert.match(
    javascriptSnippet(
      { method: "POST", path: "/k", returnsJson: false },
      target,
    ),
    /console\.log\(response\.status\);$/,
  );
});

test("Python literals use Python's constants", () => {
  assert.equal(pythonLiteral(true), "True");
  assert.equal(pythonLiteral(false), "False");
  assert.equal(pythonLiteral(null), "None");
  assert.equal(
    pythonLiteral({ a: [1, false], b: {} }),
    '{\n    "a": [\n        1,\n        False,\n    ],\n    "b": {},\n}',
  );
  const code = pythonSnippet(
    { method: "PUT", path: "/v1/y", body: { on: true }, returnsJson: false },
    target,
  );
  assert.match(code, /^import requests\n\nresponse = requests\.put\(/);
  assert.match(code, /json=\{\n {8}"on": True,\n {4}\},/);
  assert.match(code, /print\(response\.status_code\)$/);
});

test("every request language renders every example", () => {
  assert.deepEqual(REQUEST_LANGUAGES, ["curl", "javascript", "python"]);
  for (const language of REQUEST_LANGUAGES) {
    for (const example of LOCAL_API_EXAMPLES) {
      const code = requestSnippet(language, example.request, target);
      assert.ok(code.includes(target.baseUrl), `${language} ${example.id}`);
    }
  }
});

test("a one-line command is the same command", () => {
  const spec = LOCAL_API_EXAMPLES.find((e) => e.id === "runProfile").request;
  assert.equal(
    oneLine(curlSnippet(spec, target)),
    `curl -X POST http://127.0.0.1:10108/v1/profiles/PROFILE_ID/run -H "Authorization: Bearer tok" -H "Content-Type: application/json" -d '{"url":"https://example.com","headless":false}'`,
  );
  assert.equal(oneLine("echo 'a \\ b'"), "echo 'a \\ b'");
});

test("the curl body round-trips to the request body", () => {
  for (const example of LOCAL_API_EXAMPLES.filter((e) => e.request.body)) {
    const code = curlSnippet(example.request, target);
    const body = code.match(/-d '(.*)'$/)[1];
    assert.deepEqual(JSON.parse(body), example.request.body, example.id);
  }
});

test("local examples only call routes the app publishes", () => {
  const published = new Set(
    JSON.parse(
      readFileSync(new URL("../../sdk/api-paths.json", import.meta.url)),
    ).operations.map((operation) => `${operation.method} ${operation.path}`),
  );
  for (const example of LOCAL_API_EXAMPLES) {
    const { method, path, auth } = example.request;
    if (auth === false) {
      assert.equal(path, "/openapi.json", example.id);
      continue;
    }
    const route = `${method} ${path.replace(PROFILE_ID_PLACEHOLDER, "{id}")}`;
    assert.ok(published.has(route), `${example.id}: ${route}`);
  }
});

test("only the automation routes are marked paid", () => {
  assert.deepEqual(
    LOCAL_API_EXAMPLES.filter((example) => example.paid).map((e) => e.id),
    ["runProfile", "openUrl", "killProfile"],
  );
});

test("a token is shown by its ends, never whole", () => {
  assert.equal(maskToken(null), "TOKEN");
  assert.equal(maskToken("short"), "••••••••");
  assert.equal(maskToken("abcd0123456789wxyz"), "abcd…wxyz");
});

test("the client config names the server and carries the bearer", () => {
  const config = JSON.parse(mcpClientConfig("https://h/api/mcp", "k…"));
  assert.deepEqual(config.mcpServers["donut-browser"], {
    type: "http",
    url: "https://h/api/mcp",
    headers: { Authorization: "Bearer k…" },
  });
});
