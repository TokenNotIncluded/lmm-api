"use strict";
(() => {
  const status = document.getElementById("status");
  const details = document.getElementById("details");
  const preview = document.getElementById("preview");
  const approve = document.getElementById("approve");
  const reject = document.getElementById("reject");
  const path = window.location.pathname;
  let csrf = null;
  let busy = false;
  function disable() { approve.disabled = true; reject.disabled = true; }
  function failure() {
    disable();
    csrf = null;
    status.textContent = "无法读取或提交这项操作。会话、权限或预览可能已失效，请返回原会话重新发起。没有收到成功回执时，请先查询原操作状态。";
  }
  function show(confirmation) {
    if (!confirmation || typeof confirmation !== "object" || !confirmation.preview ||
        typeof confirmation.preview !== "object" || Array.isArray(confirmation.preview)) throw new Error("invalid preview");
    details.textContent = "操作：" + confirmation.tool_name + " · 版本：" + confirmation.tool_version +
      " · 有效期至：" + new Date(confirmation.expires_at_unix * 1000).toISOString();
    // Preview and all model/extension supplied text remain inert text.
    preview.textContent = JSON.stringify(confirmation.preview, null, 2);
    disable();
    switch (confirmation.state) {
      case "pending": status.textContent = "请核对预览后选择批准或拒绝。"; break;
      case "approved": status.textContent = "已批准。这里只记录同意，尚不代表业务已执行；请回到原会话查看处理结果。"; break;
      case "rejected": status.textContent = "已拒绝。"; break;
      default: throw new Error("invalid confirmation state");
    }
  }
  async function request(method, decision) {
    const headers = { Accept: "application/json" };
    if (method === "POST") {
      headers["Content-Type"] = "application/json";
      headers["x-lmm-assistant-csrf"] = csrf;
    }
    const response = await fetch(path, { method, headers, mode: "same-origin", credentials: "same-origin",
      cache: "no-store", redirect: "error", ...(decision ? { body: JSON.stringify({ decision }) } : {}) });
    if (!response.ok) throw new Error("request rejected");
    const data = await response.json();
    if (data.ok !== true) throw new Error("request failed");
    return data;
  }
  async function decide(decision) {
    if (busy || typeof csrf !== "string" || !/^[a-f0-9]{64}$/.test(csrf)) return;
    busy = true;
    disable();
    try { const data = await request("POST", decision); show(data.receipt); csrf = null; }
    catch (_) { failure(); }
    finally { busy = false; }
  }
  approve.addEventListener("click", () => { void decide("approve"); });
  reject.addEventListener("click", () => { void decide("reject"); });
  async function load() {
    if (!/^\/api\/user\/assistant\/confirmations\/[a-f0-9]{64}$/.test(path)) throw new Error("invalid path");
    const data = await request("GET");
    show(data.confirmation);
    if (data.confirmation.state === "pending") {
      if (typeof data.csrf !== "string" || !/^[a-f0-9]{64}$/.test(data.csrf)) throw new Error("invalid browser nonce");
      csrf = data.csrf;
      approve.disabled = false;
      reject.disabled = false;
    }
  }
  void load().catch(failure);
})();
