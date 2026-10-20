//! Core browser confirmation for a frozen paid-image preview. BrowserAuth lives
//! in ImageService; neither a Go bearer Session nor a caller success flag grants
//! confirmation. Cookie and origin rules follow wallet/http.rs.
use super::image_execution::{Error, ImageService, Preview};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::json;

// Browser refresh credentials are scoped to /api/user/auth. Human confirmation
// must stay below that path; Go receives only the public URL, never the cookie.
const CONFIRMATION_PATH_PREFIX: &str = "/api/user/auth/images/confirm";
pub(super) fn confirmation_path(id: &str) -> String {
    format!("{CONFIRMATION_PATH_PREFIX}/{id}")
}

#[derive(Clone)]
struct Browser {
    service: ImageService,
    origin: String,
}

pub(crate) fn browser_router_with_local_http(
    service: ImageService,
    origin: String,
    local_http: bool,
) -> Result<Router, Error> {
    validate_origin(&origin, local_http)?;
    Ok(Router::new()
        .route(&confirmation_path("{id}"), get(page).post(confirm))
        // Retired links must not fall through the whole-Core 503 fallback or
        // accidentally become confirmation endpoints again.
        .route(
            "/core/v1/images/confirm/{id}",
            get(retired_confirmation).post(retired_confirmation),
        )
        .layer(DefaultBodyLimit::max(2048))
        .layer(middleware::map_response(secure_response))
        .with_state(Browser { service, origin }))
}
async fn retired_confirmation() -> Response {
    secure(
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"confirmation_path_retired"})),
        )
            .into_response(),
    )
}
fn validate_origin(origin: &str, local_http: bool) -> Result<(), Error> {
    let u = reqwest::Url::parse(origin).map_err(|_| Error::Invalid)?;
    let local = local_http
        && u.scheme() == "http"
        && u.host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    if (u.scheme() != "https" && !local)
        || u.origin().ascii_serialization() != origin
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn cookie(h: &HeaderMap) -> Result<&str, Error> {
    let mut out = None;
    for value in h.get_all(header::COOKIE) {
        for item in value.to_str().map_err(|_| Error::Unauthorized)?.split(';') {
            if let Some((name, value)) = item.trim().split_once('=') {
                if name == "new_api_refresh" {
                    if out.is_some() || value.is_empty() {
                        return Err(Error::Unauthorized);
                    }
                    out = Some(value);
                }
            }
        }
    }
    out.ok_or(Error::Unauthorized)
}
fn check_origin(h: &HeaderMap, origin: &str) -> Result<(), Error> {
    let mut values = h.get_all(header::ORIGIN).iter();
    if values.next().and_then(|v| v.to_str().ok()) != Some(origin) || values.next().is_some() {
        return Err(Error::Forbidden);
    }
    Ok(())
}
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
fn render(p: &Preview, csrf: &str) -> String {
    let prompt = escape(&p.input.prompt);
    let model = escape(&p.input.model);
    let group = escape(&p.input.group);
    let size = escape(&p.input.size);
    let quality = escape(&p.input.quality);
    let n = escape(&p.input.n.to_string());
    let reserve = escape(&p.reservation_credits.to_string());
    let expiry = escape(&p.expires_at.to_string());
    let csrf = escape(csrf);
    let request_kind = if p.input.reference_images.is_empty() {
        "生成图片"
    } else {
        "参考图编辑"
    };
    let references = p
        .input
        .reference_images
        .iter()
        .enumerate()
        .map(|(index, image)| {
            format!(
                "<li>参考图 {}：{}，{} 字节，SHA-256 <code>{}</code></li>",
                index + 1,
                escape(&image.media_type),
                image.byte_length,
                escape(&image.sha256)
            )
        })
        .collect::<String>();
    format!(
        r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>确认付费图片请求</title></head><body><main><h1>确认付费图片请求</h1><p>请核对下面的报价。返回原工具执行后，会从你的钱包预留全额，并按这份报价扣费。</p><dl><dt>请求类型</dt><dd>{request_kind}</dd><dt>模型</dt><dd>{model}</dd><dt>计费组</dt><dd>{group}</dd><dt>尺寸</dt><dd>{size}</dd><dt>质量</dt><dd>{quality}</dd><dt>数量</dt><dd>{n}</dd><dt>完整预留金额</dt><dd><strong>{reserve} CREDIT</strong>（500000 CREDIT = 1 USD）</dd><dt>确认有效期截止</dt><dd><time id="expiry" data-unix="{expiry}">{expiry}（Unix 秒）</time></dd></dl><h2>参考图片</h2><ol>{references}</ol><h2>图片描述</h2><pre>{prompt}</pre><input type="hidden" id="csrf" value="{csrf}"><button type="button" id="confirm">确认此报价并授权钱包扣费</button><p id="status" role="status" aria-live="polite"></p></main><script>const expiry=document.getElementById('expiry');const stamp=new Date(Number(expiry.dataset.unix)*1000);if(!Number.isNaN(stamp.getTime()))expiry.textContent=stamp.toLocaleString();document.getElementById('confirm').onclick=async()=>{{const button=document.getElementById('confirm');const status=document.getElementById('status');button.disabled=true;status.textContent='正在保存确认…';try{{const response=await fetch(location.pathname,{{method:'POST',credentials:'same-origin',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{csrf:document.getElementById('csrf').value}})}});status.textContent=response.ok?'确认已保存。请返回原工具查询状态并继续执行。':'确认未完成，请重新打开此页面检查当前状态。';}}catch(error){{status.textContent='确认结果暂时未知，请返回原工具查询状态。';}}}};</script></body></html>"#
    )
}
struct HttpError(Error);
impl From<Error> for HttpError {
    fn from(error: Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let (status, code) = match self.0 {
            Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
            Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Error::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Error::Missing => (StatusCode::NOT_FOUND, "not_found"),
            Error::Conflict => (StatusCode::CONFLICT, "conflict"),
            Error::NoFunds => (StatusCode::PAYMENT_REQUIRED, "insufficient_funds"),
            Error::Pending => (StatusCode::SERVICE_UNAVAILABLE, "pending"),
            Error::Retry | Error::Storage => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        };
        secure((status, Json(json!({"error":code}))).into_response())
    }
}
fn secure(mut response: Response) -> Response {
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, value.parse().expect("static header"));
    }
    response
}
async fn secure_response(response: Response) -> Response {
    secure(response)
}
async fn page(
    State(s): State<Browser>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Response, HttpError> {
    let (preview, csrf) = s.service.confirmation_page(cookie(&h)?, &id).await?;
    Ok(secure(Html(render(&preview, &csrf)).into_response()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    csrf: String,
}
async fn confirm(
    State(s): State<Browser>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<Confirmation>,
) -> Result<Response, HttpError> {
    check_origin(&h, &s.origin)?;
    let receipt = s.service.confirm(cookie(&h)?, &id, &input.csrf).await?;
    Ok(secure(
        Json(json!({"operation_id":receipt.operation_id,"state":receipt.state})).into_response(),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::image_execution::DrawingInput;
    use super::*;
    #[test]
    fn image_confirm_cookie_requires_unique_refresh_proof() {
        let mut h = HeaderMap::new();
        assert_eq!(cookie(&h), Err(Error::Unauthorized));
        h.insert(
            header::AUTHORIZATION,
            "Bearer go-session-is-not-browser-proof".parse().unwrap(),
        );
        assert_eq!(cookie(&h), Err(Error::Unauthorized));
        h.append(
            header::COOKIE,
            "other=1; new_api_refresh=browser-proof".parse().unwrap(),
        );
        assert_eq!(cookie(&h), Ok("browser-proof"));
        h.append(
            header::COOKIE,
            "new_api_refresh=second-proof".parse().unwrap(),
        );
        assert_eq!(cookie(&h), Err(Error::Unauthorized));
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "new_api_refresh=".parse().unwrap());
        assert_eq!(cookie(&h), Err(Error::Unauthorized));
    }
    #[test]
    fn image_confirm_origin_is_exact_and_single() {
        let mut h = HeaderMap::new();
        assert_eq!(
            check_origin(&h, "https://example.com"),
            Err(Error::Forbidden)
        );
        h.insert(
            header::ORIGIN,
            "https://example.com.evil.invalid".parse().unwrap(),
        );
        assert!(check_origin(&h, "https://example.com").is_err());
        h.insert(header::ORIGIN, "https://example.com".parse().unwrap());
        assert!(check_origin(&h, "https://example.com").is_ok());
        h.append(header::ORIGIN, "https://example.com".parse().unwrap());
        assert!(check_origin(&h, "https://example.com").is_err());
    }
    #[test]
    fn image_confirm_origin_http_needs_explicit_numeric_loopback() {
        assert!(validate_origin("https://example.com", false).is_ok());
        assert!(validate_origin("http://127.0.0.1:8080", false).is_err());
        assert!(validate_origin("http://127.0.0.1:8080", true).is_ok());
        assert!(validate_origin("http://[::1]:8080", true).is_ok());
        for origin in [
            "http://localhost:8080",
            "http://10.0.0.1:8080",
            "https://example.com/",
            "https://example.com?x=1",
            "https://user:pass@example.com",
        ] {
            assert!(validate_origin(origin, true).is_err(), "{origin}");
        }
    }
    #[test]
    fn image_confirm_html_escapes_all_user_fields_outside_script() {
        let attack = "</pre><script>alert('x')</script> & \" '";
        let preview = Preview {
            operation_id: "image:id".into(),
            confirmation_path: confirmation_path("image:id"),
            preview_digest: "digest".into(),
            input: DrawingInput {
                prompt: attack.into(),
                model: attack.into(),
                group: attack.into(),
                size: attack.into(),
                quality: attack.into(),
                n: 2,
            }
            .into(),
            reservation_credits: 1234,
            expires_at: 1792499999,
            state: "previewed".into(),
        };
        let html = render(&preview, "'\"><script>alert(1)</script>");
        assert!(!html.contains(attack));
        assert!(html.contains("&lt;/pre&gt;&lt;script&gt;"));
        assert!(html.contains("1234 CREDIT"));
        assert!(html.contains("1792499999"));
        assert!(html.contains("按这份报价扣费"));
        assert_eq!(html.matches("<script>").count(), 1);
        assert!(html.contains("document.getElementById('csrf').value"));
    }
    #[test]
    fn image_confirm_secure_headers_and_unknown_json_rejection() {
        let response = HttpError(Error::Storage).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert!(serde_json::from_str::<Confirmation>(r#"{"csrf":"x","confirmed":true}"#).is_err());
    }
}
