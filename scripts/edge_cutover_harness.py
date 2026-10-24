#!/usr/bin/env python3
"""Run a localhost-only candidate edge contract against disposable HTTP backends.

This is deliberately a fixture harness, not a production Nginx configuration.
It proves route precedence, private-route denial, callback isolation, query/body
preservation, API-vs-SPA fallback, and streaming headers without opening a
non-loopback listener or touching an installed Nginx server.
"""

from __future__ import annotations

import argparse
import http.client
import json
from pathlib import Path
import socket
import subprocess
import shutil
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class BackendHandler(BaseHTTPRequestHandler):
    server_version = "edge-fixture/1"
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args) -> None:
        return

    def _reply(self, status: int, body: bytes, content_type: str = "application/json",
               headers: dict[str, str] | None = None, *, stream: bool = False) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        if stream:
            self.send_header("X-Accel-Buffering", "no")
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()

    def _record(self, method: str, body: bytes = b"") -> None:
        self.server.seen.append({
            "method": method,
            "path": self.path,
            "headers": {key.lower(): value for key, value in self.headers.items()},
            "body": body.decode("utf-8", errors="replace"),
        })

    def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
        self._record("GET")
        service = getattr(self.server, "role", "unknown")
        if service == "extensions":
            if (self.path.startswith("/api/store/")
                    or self.path.startswith("/api/user/auth/store-claim/")
                    or self.path.startswith("/api/user/payments/orders/")
                    or self.path.startswith("/api/assistant")
                    or self.path.startswith("/api/tool-market")
                    or self.path in {"/api/user-agreement", "/api/privacy-policy", "/api/refund-policy"}):
                self._reply(200, b'{"service":"extensions"}')
                return
            self._reply(404, b'{"service":"extensions","error":"not_found"}')
            return
        if self.path.startswith("/api/remote-control/v1/pi/events"):
            first = b"data: first\n\n"
            second = b"data: second\n\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Accel-Buffering", "no")
            self.send_header("Content-Length", str(len(first) + len(second)))
            self.end_headers()
            self.wfile.write(first)
            self.wfile.flush()
            time.sleep(0.18)
            self.wfile.write(second)
            self.wfile.flush()
            return
        if self.path.startswith("/health/ready"):
            self._reply(200, b'{"service":"core","ready":true}')
            return
        if self.path.startswith("/v1/models"):
            self._reply(200, b'{"service":"core","models":[]}')
            return
        if self.path.startswith("/api/oauth"):
            query = self.path.split("?", 1)[1] if "?" in self.path else ""
            self._reply(200, json.dumps({"service": "core", "query": query}).encode())
            return
        if self.path.startswith("/api/user/"):
            self._reply(200, b'{"service":"core","route":"identity"}')
            return
        self._reply(404, b'{"service":"core","error":"not_found"}')

    def do_HEAD(self) -> None:  # noqa: N802 - stdlib handler API
        self._record("HEAD")
        self._reply(200, b"")

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler API
        size = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(size)
        self._record("POST", body)
        if getattr(self.server, "role", "unknown") == "extensions" and self.path.startswith("/callbacks/"):
            self._reply(202, json.dumps({"service": "extensions", "body": body.decode()}).encode())
            return
        self._reply(404, json.dumps({"service": getattr(self.server, "role", "unknown"), "error": "not_found"}).encode())


def _serve(server: ThreadingHTTPServer) -> None:
    server.serve_forever(poll_interval=0.02)


def _free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _request(port: int, method: str, path: str, body: bytes = b"", headers: dict[str, str] | None = None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=4)
    connection.request(method, path, body=body, headers=headers or {})
    response = connection.getresponse()
    payload = response.read()
    result = (response.status, {key.lower(): value for key, value in response.getheaders()}, payload)
    connection.close()
    return result


def _stream_first_chunk(port: int) -> tuple[int, dict[str, str], bytes, float]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=4)
    started = time.monotonic()
    connection.request("GET", "/api/remote-control/v1/pi/events")
    response = connection.getresponse()
    first = response.read(len(b"data: first\n\n"))
    elapsed = time.monotonic() - started
    headers = {key.lower(): value for key, value in response.getheaders()}
    response.read()
    connection.close()
    return response.status, headers, first, elapsed


def _nginx_config(root: Path, edge_port: int, core_port: int, extension_port: int) -> Path:
    (root / "index.html").write_text("<html><body>fixture-spa</body></html>\n", encoding="utf-8")
    (root / "static").mkdir()
    (root / "static" / "app.123456.js").write_text("fixture", encoding="utf-8")
    (root / "nginx.conf").write_text(
        "pid " + str(root / "nginx.pid") + ";\n"
        "error_log " + str(root / "error.log") + ";\n"
        "daemon off;\nmaster_process off;\nevents {}\n"
        "http {\n"
        "include /etc/nginx/mime.types;\n"
        "access_log off;\n"
        "client_body_temp_path " + str(root / "body") + ";\n"
        "proxy_temp_path " + str(root / "proxy") + ";\n"
        "fastcgi_temp_path " + str(root / "fastcgi") + ";\n"
        "uwsgi_temp_path " + str(root / "uwsgi") + ";\n"
        "scgi_temp_path " + str(root / "scgi") + ";\n"
        "upstream fixture_core { server 127.0.0.1:" + str(core_port) + "; }\n"
        "upstream fixture_extensions { server 127.0.0.1:" + str(extension_port) + "; }\n"
        "server {\nlisten 127.0.0.1:" + str(edge_port) + ";\n"
        "root " + str(root) + ";\n"
        "proxy_http_version 1.1;\n"
        "location = /health/ready { proxy_set_header X-LMM-Edge-Proof fixture-proof; proxy_set_header X-LMM-Edge-Client-IP $remote_addr; proxy_pass http://fixture_core; }\n"
        "location ^~ /v1/ { proxy_set_header X-LMM-Edge-Proof fixture-proof; proxy_set_header X-LMM-Edge-Client-IP $remote_addr; proxy_pass http://fixture_core; }\n"
        "location ^~ /api/remote-control/v1/pi/ { proxy_buffering off; proxy_read_timeout 30s; proxy_set_header X-LMM-Edge-Proof fixture-proof; proxy_set_header X-LMM-Edge-Client-IP $remote_addr; proxy_pass http://fixture_core; }\n"
        "location ^~ /api/oauth { proxy_set_header X-LMM-Edge-Proof fixture-proof; proxy_set_header X-LMM-Edge-Client-IP $remote_addr; proxy_pass http://fixture_core; }\n"
        "location ^~ /api/user/ { proxy_set_header X-LMM-Edge-Proof fixture-proof; proxy_set_header X-LMM-Edge-Client-IP $remote_addr; proxy_pass http://fixture_core; }\n"
        "location ^~ /api/store/ { proxy_set_header X-LMM-Edge-Proof \"\"; proxy_set_header X-LMM-Edge-Client-IP \"\"; proxy_set_header Authorization \"\"; proxy_pass http://fixture_extensions; }\n"
        "location ^~ /api/user/auth/store-claim/ { proxy_set_header X-LMM-Edge-Proof \"\"; proxy_set_header X-LMM-Edge-Client-IP \"\"; proxy_set_header Authorization \"\"; proxy_pass http://fixture_extensions; }\n"
        "location ^~ /api/user/payments/orders/ { proxy_set_header X-LMM-Edge-Proof \"\"; proxy_set_header X-LMM-Edge-Client-IP \"\"; proxy_set_header Authorization \"\"; proxy_pass http://fixture_extensions; }\n"
        "location ^~ /api/assistant { proxy_set_header X-LMM-Edge-Proof \"\"; proxy_set_header X-LMM-Edge-Client-IP \"\"; proxy_set_header Authorization \"\"; proxy_pass http://fixture_extensions; }\n"
        "location ^~ /api/tool-market { proxy_set_header X-LMM-Edge-Proof \"\"; proxy_set_header X-LMM-Edge-Client-IP \"\"; proxy_set_header Authorization \"\"; proxy_pass http://fixture_extensions; }\n"
        "location ^~ /callbacks/ { proxy_set_header X-LMM-Edge-Proof \"\"; proxy_set_header X-LMM-Edge-Client-IP \"\"; proxy_set_header Authorization \"\"; proxy_pass http://fixture_extensions; }\n"
        "location = /api/user-agreement { proxy_pass http://fixture_extensions; }\n"
        "location = /api/privacy-policy { proxy_pass http://fixture_extensions; }\n"
        "location = /api/refund-policy { proxy_pass http://fixture_extensions; }\n"
        "location ^~ /internal/ { return 404; }\n"
        "location ^~ /extensions/v1/ { return 404; }\n"
        "location ^~ /api/ { return 404; }\n"
        "location ^~ /static/ { try_files $uri =404; add_header Cache-Control \"public, max-age=31536000, immutable\" always; }\n"
        "location / { try_files $uri $uri/ /index.html; add_header Cache-Control \"no-cache\" always; }\n"
        "}\n}\n",
        encoding="utf-8",
    )
    for directory in (root / "body", root / "proxy", root / "fastcgi", root / "uwsgi", root / "scgi"):
        directory.mkdir()
    return root / "nginx.conf"


def run_harness() -> dict[str, object]:
    nginx = shutil.which("nginx")
    if nginx is None:
        raise RuntimeError("nginx executable is required for the local edge harness")
    core = ThreadingHTTPServer(("127.0.0.1", 0), BackendHandler)
    extensions = ThreadingHTTPServer(("127.0.0.1", 0), BackendHandler)
    core.role = "core"
    extensions.role = "extensions"
    core.seen = []
    extensions.seen = []
    core_thread = threading.Thread(target=_serve, args=(core,), daemon=True)
    extensions_thread = threading.Thread(target=_serve, args=(extensions,), daemon=True)
    core_thread.start()
    extensions_thread.start()
    edge_port = _free_port()
    nginx_process = None
    try:
        with tempfile.TemporaryDirectory(prefix="lmm-edge-harness-") as temporary:
            root = Path(temporary)
            config = _nginx_config(root, edge_port, core.server_port, extensions.server_port)
            check = subprocess.run([nginx, "-t", "-p", str(root), "-c", str(config)], capture_output=True, text=True, timeout=5)
            if check.returncode != 0:
                raise AssertionError("nginx -t failed: " + check.stderr)
            nginx_process = subprocess.Popen([nginx, "-p", str(root), "-c", str(config)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            deadline = time.monotonic() + 4
            while time.monotonic() < deadline:
                try:
                    if _request(edge_port, "GET", "/health/ready")[0] == 200:
                        break
                except OSError:
                    time.sleep(0.03)
            else:
                raise AssertionError("fixture nginx did not start")

            status, headers, body = _request(edge_port, "GET", "/health/ready")
            assert (status, json.loads(body)["service"]) == (200, "core")
            assert core.seen[-1]["headers"].get("x-lmm-edge-proof") == "fixture-proof"
            status, _, body = _request(edge_port, "GET", "/api/store/item")
            assert (status, json.loads(body)["service"]) == (200, "extensions")
            status, _, body = _request(edge_port, "GET", "/api/user/profile")
            assert (status, json.loads(body)["service"]) == (200, "core")
            status, _, body = _request(edge_port, "GET", "/api/assistant/status")
            assert (status, json.loads(body)["service"]) == (200, "extensions")
            status, _, body = _request(edge_port, "POST", "/callbacks/epay", b"signed=fixture", {"Authorization": "Bearer host-secret", "X-LMM-Edge-Proof": "caller-proof", "Content-Type": "application/x-www-form-urlencoded"})
            assert (status, json.loads(body)["service"]) == (202, "extensions")
            callback = extensions.seen[-1]
            assert callback["body"] == "signed=fixture"
            assert "authorization" not in callback["headers"]
            assert "x-lmm-edge-proof" not in callback["headers"]
            status, _, body = _request(edge_port, "GET", "/api/oauth/google?state=fixture&code=opaque")
            assert (status, json.loads(body)["service"]) == (200, "core")
            assert json.loads(body)["query"] == "state=fixture&code=opaque"
            status, headers, body = _request(edge_port, "GET", "/internal/access-ip-policy")
            assert status == 404 and b"fixture-spa" not in body
            status, headers, body = _request(edge_port, "GET", "/extensions/v1/private")
            assert status == 404 and b"fixture-spa" not in body
            status, headers, body = _request(edge_port, "GET", "/api/unknown")
            assert status == 404 and b"fixture-spa" not in body
            status, headers, body = _request(edge_port, "GET", "/static/app.123456.js")
            assert status == 200 and headers.get("cache-control", "").startswith("public")
            status, headers, body = _request(edge_port, "GET", "/unknown")
            assert status == 200 and body == b"<html><body>fixture-spa</body></html>\n"
            status, headers, body, elapsed = _stream_first_chunk(edge_port)
            assert status == 200 and headers.get("content-type", "").startswith("text/event-stream")
            assert body == b"data: first\n\n", (headers, body)
            assert elapsed < 0.15, f"SSE first chunk was buffered ({elapsed:.3f}s)"
            return {"status": "PASS", "nginx_test": True, "checks": 12, "listen": "127.0.0.1"}
    finally:
        if nginx_process is not None and nginx_process.poll() is None:
            nginx_process.terminate()
            try:
                nginx_process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                nginx_process.kill()
        core.shutdown()
        extensions.shutdown()
        core.server_close()
        extensions.server_close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", type=Path, help="write a sanitized result file")
    args = parser.parse_args()
    result = run_harness()
    rendered = json.dumps(result, sort_keys=True) + "\n"
    if args.json:
        args.json.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
