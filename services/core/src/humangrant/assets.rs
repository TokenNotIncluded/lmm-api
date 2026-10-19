//! Static Web assets and HTML page rendering for HumanGrant.

pub const CLIENT_JS: &[u8] = include_bytes!("web/client.mjs");
pub const UI_JS: &[u8] = include_bytes!("web/ui.mjs");
pub const STYLE_CSS: &[u8] = include_bytes!("web/style.css");

pub fn render_page(mode: &str, data: &serde_json::Value) -> String {
    let payload = serde_json::json!({
        "mode": mode,
        "data": data,
    });
    let escaped_json = serde_json::to_string(&payload)
        .unwrap_or_else(|_| "{}".to_string())
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>HumanGrant</title><link rel="stylesheet" href="/humangrant/style.css"><script type="module" src="/humangrant/ui.js"></script></head>
<body><main><header><a href="/">LMM</a><a href="/human/grants">My AI grants</a></header><h1>HumanGrant</h1><p id="notice" role="status" aria-live="polite"></p><div id="app"></div><pre hidden id="hg-data">{}</pre></main></body></html>"#,
        escaped_json
    )
}
