// UI unit boundary only: real Session/provider/PG/browser acceptance is separate.
"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const code = fs.readFileSync(__dirname + "/confirmation.js", "utf8");
const id = "a".repeat(64), nonce = "b".repeat(64);
const path = "/api/user/assistant/confirmations/" + id;
const evil = "</pre><img src=x onerror=alert(1)><script>alert(2)</script>\u2028'\"&";
function confirmation(state = "pending") {
  return { tool_name: evil, tool_version: "main-3948a0a1", preview: { title: evil, body: "exact\nmessage" }, expires_at_unix: 2000000000, state, executed: false };
}
async function flush() { for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); }
function ui(replies, location = path) {
  const nodes = {};
  for (const name of ["status", "details", "preview", "approve", "reject"]) {
    nodes[name] = { disabled: true, textContent: "", listeners: {}, addEventListener(name, callback) { this.listeners[name] = callback; } };
    Object.defineProperty(nodes[name], "innerHTML", { set() { throw new Error("unsafe HTML sink"); } });
  }
  const requests = [];
  const context = {
    document: { getElementById(name) { assert.ok(nodes[name]); return nodes[name]; } },
    window: { location: { pathname: location } },
    async fetch(url, options) {
      assert.equal(url, path);
      assert.equal(options.mode, "same-origin");
      assert.equal(options.credentials, "same-origin");
      assert.equal(options.redirect, "error");
      assert.equal(options.cache, "no-store");
      requests.push({ url, ...options });
      const reply = replies.shift();
      if (reply instanceof Error) throw reply;
      assert.ok(reply, "unexpected extra request");
      return { ok: reply.status !== false, async json() { return reply.body; } };
    },
  };
  vm.runInNewContext(code, context);
  return { nodes, requests, click(name) { nodes[name].listeners.click(); } };
}
test("malicious preview remains exact inert text; load never approves; double click submits once", async () => {
  const view = ui([{ body: { ok: true, confirmation: confirmation(), csrf: nonce } }, { body: { ok: true, receipt: confirmation("approved"), executed: false } }]);
  await flush();
  assert.equal(view.requests.length, 1);
  assert.equal(view.requests[0].method, "GET");
  assert.equal(view.nodes.preview.textContent, JSON.stringify(confirmation().preview, null, 2));
  assert.ok(view.nodes.details.textContent.includes(evil));
  assert.equal(view.nodes.approve.disabled, false);
  view.click("approve"); view.click("approve"); await flush();
  assert.equal(view.requests.length, 2);
  assert.deepEqual(JSON.parse(view.requests[1].body), { decision: "approve" });
  assert.equal(view.requests[1].headers["x-lmm-assistant-csrf"], nonce);
  assert.ok(view.nodes.status.textContent.includes("尚不代表业务已执行"));
  assert.equal(view.nodes.approve.disabled, true);
});
test("reject uses the same real browser nonce and closed decision", async () => {
  const view = ui([{ body: { ok: true, confirmation: confirmation(), csrf: nonce } }, { body: { ok: true, receipt: confirmation("rejected") } }]);
  await flush(); view.click("reject"); await flush();
  assert.deepEqual(JSON.parse(view.requests[1].body), { decision: "reject" });
  assert.equal(view.nodes.status.textContent, "已拒绝。");
});
test("missing nonce, unauthenticated response and invalid path never enable approval", async () => {
  for (const replies of [[{ body: { ok: true, confirmation: confirmation() } }], [{ status: false }], [{ body: { ok: false } }]]) {
    const view = ui(replies); await flush(); view.click("approve"); await flush();
    assert.equal(view.requests.length, 1);
    assert.equal(view.nodes.approve.disabled, true);
  }
  const view = ui([], "/api/user/assistant/confirmations/../../other"); await flush();
  assert.equal(view.requests.length, 0);
  assert.equal(view.nodes.approve.disabled, true);
});
test("an uncertain POST is not retried or reported executed", async () => {
  const view = ui([{ body: { ok: true, confirmation: confirmation(), csrf: nonce } }, new Error("lost ACK")]);
  await flush(); view.click("approve"); await flush(); view.click("approve"); await flush();
  assert.equal(view.requests.length, 2);
  assert.ok(view.nodes.status.textContent.includes("查询原操作状态"));
  assert.equal(view.nodes.approve.disabled, true);
});
