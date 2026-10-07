import assert from "node:assert/strict";
import test from "node:test";
import {
  filterTableRows,
  resolveTableVisibility,
  selectTableRange,
} from "./table-tools.ts";

const rows = [
  { id: "alpha", name: "Client North", state: "stopped", tags: ["work", "eu"] },
  { id: "beta", name: "Client South", state: "running", tags: ["work"] },
  { id: "gamma", name: "Personal", state: "stopped", tags: ["eu"] },
];
const filters = [
  { id: "state", value: (row) => row.state },
  { id: "tag", value: (row) => row.tags },
];
const search = (row) => row.name;

test("search and facets intersect; values in a single facet form alternatives", () => {
  const result = filterTableRows(rows, "CLIENT", search, filters, {
    state: ["stopped", "running"],
    tag: ["eu"],
  });
  assert.deepEqual(
    result.map((row) => row.id),
    ["alpha"],
  );
  assert.equal(
    filterTableRows(rows, "client south", search, filters, {}).length,
    1,
  );
  assert.equal(
    filterTableRows(rows, "personal", search, filters, { tag: ["work"] })
      .length,
    0,
  );
});

test("facet counts exclude their own filter but retain all other constraints", () => {
  assert.deepEqual(
    filterTableRows(
      rows,
      "client",
      search,
      filters,
      { state: ["stopped"], tag: ["work"] },
      "state",
    ).map((row) => row.id),
    ["alpha", "beta"],
  );
});

test("range selection follows the displayed order and skips unavailable rows", () => {
  const eligible = ["gamma", "alpha", "beta"];
  assert.deepEqual(
    selectTableRange(eligible, {}, "gamma", "beta", true, true),
    { gamma: true, alpha: true, beta: true },
  );
  assert.deepEqual(
    selectTableRange(["gamma", "beta"], {}, "gamma", "beta", true, true),
    { gamma: true, beta: true },
  );
  assert.deepEqual(
    selectTableRange(
      eligible,
      { alpha: true, beta: true, gamma: true },
      "beta",
      "alpha",
      false,
      true,
    ),
    { gamma: true },
  );
});

test("a removed range anchor becomes a single selection and a removed target cannot be selected", () => {
  assert.deepEqual(
    selectTableRange(["beta"], {}, "alpha", "beta", true, true),
    { beta: true },
  );
  assert.deepEqual(
    selectTableRange(["beta"], { beta: true }, "beta", "alpha", true, true),
    { beta: true },
  );
});

test("column preferences override responsive defaults but cannot hide identity or reveal unavailable features", () => {
  assert.deepEqual(
    resolveTableVisibility(
      { note: false, tags: true },
      { note: true, tags: false, name: false, bot: true },
      ["name"],
      ["bot"],
    ),
    { note: true, tags: false, name: true, bot: false },
  );
});
