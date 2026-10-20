//! Real HTTP login/confirmation with Python's standard CookieJar. This checks
//! Set-Cookie and automatic path matching, not browser SameSite enforcement or
//! browser acceptance. No refresh cookie is copied into a request by this test.
use super::*;
use rand::{RngCore, rngs::OsRng};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
};

struct PrivateJar {
    root: PathBuf,
    path: PathBuf,
}
impl PrivateJar {
    fn new() -> Self {
        let mut random = [0; 16];
        OsRng.try_fill_bytes(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!("lmm-image-http-cookie-{}", hex(&random)));
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        let path = root.join("cookies.lwp");
        let jar = Self { root, path };
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&jar.path)
            .unwrap();
        jar
    }
    fn assert_private(&self) {
        assert_eq!(
            fs::metadata(&self.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&self.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
impl Drop for PrivateJar {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct HttpServer(tokio::task::JoinHandle<()>);
impl Drop for HttpServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn python(script: &'static str, input: Value) -> Value {
    // Access tokens, CSRF and response bodies stay in the captured pipes and
    // process memory. Do not attach child output to assertions or test logs.
    tokio::task::spawn_blocking(move || {
        let mut child = Command::new("python3")
            .args(["-I", "-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Python standard-library HTTP fixture is available");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&input).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "HTTP CookieJar subprocess failed; private output withheld"
        );
        serde_json::from_slice(&output.stdout)
            .expect("HTTP CookieJar subprocess returns private JSON")
    })
    .await
    .unwrap()
}

const LOGIN: &str = r#"
import http.cookiejar
import http.cookies
import json
import sys
import urllib.request

config = json.load(sys.stdin)
jar = http.cookiejar.LWPCookieJar(config['jar'])
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(jar))
request = urllib.request.Request(
    config['origin'] + '/api/user/login',
    data=json.dumps({'username': 'drawingowner', 'password': 'drawing-password'}).encode(),
    headers={'Content-Type': 'application/json', 'Origin': config['origin'],
             'User-Agent': 'synthetic drawing CookieJar PostgreSQL test'},
    method='POST')
with opener.open(request, timeout=10) as response:
    status = response.status
    data = json.loads(response.read(131073))
    headers = response.headers.get_all('Set-Cookie', [])
cookies = []
for value in headers:
    parsed = http.cookies.SimpleCookie()
    parsed.load(value)
    if 'new_api_refresh' in parsed:
        cookies.append(parsed['new_api_refresh'])
refresh = [cookie for cookie in jar if cookie.name == 'new_api_refresh']
jar.save(ignore_discard=True, ignore_expires=True)
cookie = cookies[0] if len(cookies) == 1 else None
print(json.dumps({
    'status': status,
    'access': data.get('data', {}).get('access_token'),
    'refresh_set_cookie_count': len(cookies),
    'path': cookie['path'] if cookie else None,
    'http_only': bool(cookie and cookie['httponly']),
    'secure': bool(cookie and cookie['secure']),
    'same_site': cookie['samesite'] if cookie else None,
    'domain_attribute_absent': bool(cookie and not cookie['domain']),
    'stored_refresh_count': len(refresh),
    'stored_path': refresh[0].path if len(refresh) == 1 else None,
    'stored_host_only': len(refresh) == 1 and not refresh[0].domain_specified,
    'stored_secure': len(refresh) == 1 and refresh[0].secure,
}))
"#;

const NEGATIVE_REQUESTS: &str = r#"
import html.parser
import http.cookiejar
import json
import sys
import urllib.error
import urllib.request

config = json.load(sys.stdin)
jar = http.cookiejar.LWPCookieJar(config['jar'])
jar.load(ignore_discard=True, ignore_expires=True)
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(jar))
bare = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def automatically_sends_refresh(path):
    request = urllib.request.Request(config['origin'] + path)
    jar.add_cookie_header(request)
    return any(item.strip().startswith('new_api_refresh=')
               for item in request.get_header('Cookie', '').split(';'))

def request(client, path, *, csrf=None, origin=None, bearer=False):
    headers = {}
    if origin is not None:
        headers['Origin'] = origin
    if bearer:
        headers['Authorization'] = 'Bearer ' + config['access']
    data = None
    if csrf is not None:
        headers['Content-Type'] = 'application/json'
        data = json.dumps({'csrf': csrf}).encode()
    message = urllib.request.Request(config['origin'] + path, data=data, headers=headers,
                                     method='POST' if data is not None else 'GET')
    try:
        response = client.open(message, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read(131073)
        if len(body) > 131072:
            raise RuntimeError('HTTP fixture response exceeds bound')
        return response.status, response.headers, body.decode('utf-8')

class Confirmation(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.csrf = ''
        self.prompt = ''
        self.in_prompt = False
    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'input' and attrs.get('id') == 'csrf':
            self.csrf = attrs.get('value', '')
        if tag == 'pre':
            self.in_prompt = True
    def handle_endtag(self, tag):
        if tag == 'pre':
            self.in_prompt = False
    def handle_data(self, data):
        if self.in_prompt:
            self.prompt += data

path = config['path']
old_path = '/core/v1/images/confirm/' + config['operation_id']
scope = {
    'new_confirmation': automatically_sends_refresh(path),
    'auth_refresh': automatically_sends_refresh('/api/user/auth/refresh'),
    'old_confirmation': automatically_sends_refresh(old_path),
    'lookalike_prefix': automatically_sends_refresh(
        '/api/user/authentication/images/confirm/' + config['operation_id']),
    'login': automatically_sends_refresh('/api/user/login'),
}
old_status, old_headers, _ = request(opener, old_path)
old_post_status, old_post_headers, _ = request(
    opener, old_path, csrf='0' * 64, origin=config['origin'])
missing_status, _, _ = request(bare, path)
bearer_status, _, _ = request(bare, path, bearer=True)
page_status, page_headers, body = request(opener, path)
page = Confirmation()
page.feed(body)
csrf = page.csrf
wrong_csrf = (('0' if csrf[:1] != '0' else '1') + csrf[1:]
              if len(csrf) == 64 else '0' * 64)
wrong_origin_status, _, _ = request(
    opener, path, csrf=csrf, origin='https://other.example.invalid')
wrong_csrf_status, _, _ = request(opener, path, csrf=wrong_csrf, origin=config['origin'])
missing_post_status, _, _ = request(bare, path, csrf=csrf, origin=config['origin'])
bearer_post_status, _, _ = request(
    bare, path, csrf=csrf, origin=config['origin'], bearer=True)
print(json.dumps({
    'scope': scope,
    'old_status': old_status,
    'old_no_store': old_headers.get('Cache-Control') == 'no-store',
    'old_post_status': old_post_status,
    'old_post_no_store': old_post_headers.get('Cache-Control') == 'no-store',
    'missing_status': missing_status,
    'bearer_status': bearer_status,
    'page_status': page_status,
    'page_no_store': page_headers.get('Cache-Control') == 'no-store',
    'prompt_matches': page.prompt == config['prompt'],
    'prompt_escaped': '&lt;scene&gt;&amp;light&gt;' in body and '<scene>' not in body,
    'quote_visible': all(value in body for value in (
        '6000 CREDIT', 'dall-e-3', 'allowed', '1024x1024', 'standard')),
    'csrf_bounded_hex': len(csrf) == 64 and all(c in '0123456789abcdef' for c in csrf),
    'wrong_origin_status': wrong_origin_status,
    'wrong_csrf_status': wrong_csrf_status,
    'missing_post_status': missing_post_status,
    'bearer_post_status': bearer_post_status,
    'csrf': csrf,
}))
"#;

const CONFIRM: &str = r#"
import http.cookiejar
import json
import sys
import urllib.error
import urllib.request

config = json.load(sys.stdin)
jar = http.cookiejar.LWPCookieJar(config['jar'])
jar.load(ignore_discard=True, ignore_expires=True)
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(jar))
request = urllib.request.Request(
    config['origin'] + config['path'],
    data=json.dumps({'csrf': config['csrf']}).encode(),
    headers={'Content-Type': 'application/json', 'Origin': config['origin']},
    method='POST')
try:
    response = opener.open(request, timeout=10)
except urllib.error.HTTPError as error:
    response = error
with response:
    body = response.read(131073)
    if len(body) > 131072:
        raise RuntimeError('HTTP fixture response exceeds bound')
    data = json.loads(body)
    print(json.dumps({'status': response.status,
                      'no_store': response.headers.get('Cache-Control') == 'no-store',
                      'state': data.get('state'),
                      'operation_id': data.get('operation_id')}))
"#;

#[sqlx::test(migrations = false)]
async fn image_drawing_http_cookie_jar_scopes_confirmation_without_bearer_or_money_dispatch(
    pool: PgPool,
) {
    let f = fixture(pool).await;
    let supplier = Supplier::start(Reply::OneImage).await;
    let service = image_service_for_pool(&f.pool, &supplier);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let auth = crate::legacy_auth::LegacyAuthConfig::new(&origin, &[43; 32])
        .unwrap()
        .with_insecure_local_http()
        .unwrap()
        .with_legal_acceptance_required(false);
    let app = crate::legacy_auth::router(f.identity.clone(), auth).merge(
        crate::model_http::image_http::browser_router_with_local_http(
            service.clone(),
            origin.clone(),
            true,
        )
        .unwrap(),
    );
    let _server = HttpServer(tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    }));
    let jar = PrivateJar::new();
    jar.assert_private();
    let login = python(LOGIN, json!({"origin":origin,"jar":jar.path})).await;
    assert_eq!(login["status"], 200);
    assert_eq!(login["refresh_set_cookie_count"], 1);
    assert_eq!(login["path"], "/api/user/auth");
    assert_eq!(login["http_only"], true);
    assert_eq!(login["secure"], false);
    assert_eq!(login["same_site"], "Strict");
    assert_eq!(login["domain_attribute_absent"], true);
    assert_eq!(login["stored_refresh_count"], 1);
    assert_eq!(login["stored_path"], "/api/user/auth");
    assert_eq!(login["stored_host_only"], true);
    assert_eq!(login["stored_secure"], false);
    jar.assert_private();
    let access = login["access"]
        .as_str()
        .expect("real HTTP login issues a Session access token");
    assert!(!access.is_empty());
    assert!(access != f.secret);
    let actor = f.identity.authorize(access).await.unwrap();
    assert_eq!(actor.credential_kind, CredentialKind::Session);
    assert_eq!(actor.user_id, f.user);
    let raw = drawing_input();
    let builtin = builtin_with_session(
        &f.pool,
        access,
        "drawing-tool",
        "drawing-test-v1",
        "drawing.generate",
        &"a".repeat(64),
        &crate::builtins::input_digest(&raw).unwrap(),
    )
    .await;
    let preview = service
        .preview(access, &builtin.context, raw.clone())
        .await
        .unwrap();
    assert_eq!(preview.state, "previewed");
    assert_eq!(preview.reservation_credits, 6000);
    assert_eq!(
        preview.confirmation_path,
        format!("/api/user/auth/images/confirm/{}", preview.operation_id)
    );
    assert!(preview.operation_id.starts_with("image:"));
    assert_eq!(preview.operation_id.len(), 70);
    no_model_hold(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);

    let negative = python(
        NEGATIVE_REQUESTS,
        json!({
            "origin":origin,"jar":jar.path,"access":access,
            "path":preview.confirmation_path,"operation_id":preview.operation_id,
            "prompt":raw["prompt"],
        }),
    )
    .await;
    assert_eq!(negative["scope"]["new_confirmation"], true);
    assert_eq!(negative["scope"]["auth_refresh"], true);
    for path in ["old_confirmation", "lookalike_prefix", "login"] {
        assert_eq!(negative["scope"][path], false, "cookie path: {path}");
    }
    assert_eq!(negative["old_status"], 404);
    assert_eq!(negative["old_no_store"], true);
    assert_eq!(negative["old_post_status"], 404);
    assert_eq!(negative["old_post_no_store"], true);
    assert_eq!(negative["missing_status"], 401);
    assert_eq!(negative["bearer_status"], 401);
    assert_eq!(negative["page_status"], 200);
    for proof in [
        "page_no_store",
        "prompt_matches",
        "prompt_escaped",
        "quote_visible",
        "csrf_bounded_hex",
    ] {
        assert_eq!(negative[proof], true, "confirmation page: {proof}");
    }
    assert_eq!(negative["wrong_origin_status"], 403);
    assert_eq!(negative["wrong_csrf_status"], 403);
    assert_eq!(negative["missing_post_status"], 401);
    assert_eq!(negative["bearer_post_status"], 401);
    let unchanged = service
        .query_for_context(access, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(unchanged.state, "previewed");
    assert!(unchanged.output.is_none());
    no_model_hold(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);

    let confirmed = python(
        CONFIRM,
        json!({"origin":origin,"jar":jar.path,"path":preview.confirmation_path,
               "csrf":negative["csrf"]}),
    )
    .await;
    assert_eq!(confirmed["status"], 200);
    assert_eq!(confirmed["no_store"], true);
    assert_eq!(confirmed["state"], "confirmed");
    assert_eq!(confirmed["operation_id"], preview.operation_id);
    let receipt = service
        .query_for_context(access, &preview.operation_id, &builtin.context)
        .await
        .unwrap();
    assert_eq!(receipt.state, "confirmed");
    assert!(receipt.output.is_none());
    assert!(receipt.charged_credits.is_none());
    // Browser confirmation grants permission only. Explicit Core execution is
    // deliberately absent, so no supplier POST or model hold may have happened.
    no_model_hold(&f).await;
    assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    jar.assert_private();
    let private_root = jar.root.clone();
    drop(jar);
    assert!(!private_root.exists());
}
