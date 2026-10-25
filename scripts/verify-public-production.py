#!/usr/bin/env python3
"""Read-only acceptance for the public origin; never confirms a native transaction."""
import argparse
from dataclasses import dataclass, field
from html.parser import HTMLParser
import json
import re
import ssl
import time
import urllib.parse
import urllib.request

ORIGIN = "https://api.lmm.best"
MAX_BODY = 8 * 1024 * 1024


@dataclass(frozen=True)
class Response:
    status: int
    content_type: str
    body: bytes
    headers: dict = field(default_factory=dict)


class Assets(HTMLParser):
    def __init__(self):
        super().__init__()
        self.urls = set()

    def handle_starttag(self, tag, attributes):
        values = dict(attributes)
        url = values.get("src") if tag == "script" else values.get("href") if tag == "link" else None
        if url and re.search(r"\.(?:js|css)$", urllib.parse.urlsplit(url).path):
            self.urls.add(url)


class SameOriginRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        if urllib.parse.urlsplit(new_url).netloc != urllib.parse.urlsplit(ORIGIN).netloc:
            raise ValueError("public probe refused a cross-origin redirect")
        if urllib.parse.urlsplit(new_url).scheme != "https":
            raise ValueError("public probe refused a non-HTTPS redirect")
        return super().redirect_request(request, fp, code, message, headers, new_url)


def response_header(response, name):
    wanted = name.lower()
    for key, value in response.headers.items():
        if key.lower() == wanted:
            return value
    return None


def verify(fetch, expected_version=None):
    status = fetch(ORIGIN + "/api/status")
    if status.status != 200 or "json" not in status.content_type:
        raise ValueError("status endpoint did not return HTTP 200 JSON")
    payload = json.loads(status.body)
    data = payload.get("data") or {}
    if payload.get("success") is not True or not isinstance(data.get("version"), str):
        raise ValueError("status endpoint is not a healthy versioned response")
    if expected_version and data["version"] != expected_version:
        raise ValueError(f"backend version mismatch: expected {expected_version}, got {data['version']}")
    for name in (
        "x-release-commit",
        "x-core-artifact-sha256",
        "x-extensions-artifact-sha256",
        "x-schema-fingerprint",
        "x-route-identity",
    ):
        if response_header(status, name) is not None:
            raise ValueError(f"public status exposes private release identity: {name}")
    request_id = response_header(status, "x-request-id") or response_header(status, "x-oneapi-request-id")
    if not request_id or not re.fullmatch(r"[A-Za-z0-9._:-]{1,256}", request_id):
        raise ValueError("public status response has no usable request ID")
    assets = set()
    for path in ("/", "/login", "/console"):
        response = fetch(ORIGIN + path)
        if response.status != 200 or "text/html" not in response.content_type:
            raise ValueError(f"public page {path} is not HTTP 200 HTML")
        parser = Assets()
        parser.feed(response.body.decode("utf-8"))
        local = set()
        for raw in parser.urls:
            url = urllib.parse.urljoin(ORIGIN + "/", raw)
            parsed = urllib.parse.urlsplit(url)
            if parsed.scheme == "https" and parsed.netloc == urllib.parse.urlsplit(ORIGIN).netloc:
                local.add(urllib.parse.urlunsplit(parsed._replace(fragment="")))
        if not any(urllib.parse.urlsplit(url).path.endswith(".js") for url in local):
            raise ValueError(f"public page {path} has no local JavaScript entry")
        if not any(urllib.parse.urlsplit(url).path.endswith(".css") for url in local):
            raise ValueError(f"public page {path} has no local stylesheet")
        assets.update(local)
    if len(assets) > 64:
        raise ValueError("public page references too many assets")
    total = 0
    for url in sorted(assets):
        response = fetch(url)
        kind = "css" if urllib.parse.urlsplit(url).path.endswith(".css") else "javascript"
        if response.status != 200 or not response.body or kind not in response.content_type:
            raise ValueError(f"asset is missing or served as a SPA fallback: {urllib.parse.urlsplit(url).path}")
        if len(response.body) > MAX_BODY:
            raise ValueError("public asset exceeds the probe size limit")
        total += len(response.body)
        if total > 64 * 1024 * 1024:
            raise ValueError("public assets exceed the total probe size limit")
    return {"origin": ORIGIN, "backend_version": data["version"], "pages": 3, "assets": len(assets), "asset_bytes": total}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-backend-version")
    args = parser.parse_args()
    opener = urllib.request.build_opener(SameOriginRedirect(), urllib.request.HTTPSHandler(context=ssl.create_default_context()))
    deadline = time.monotonic() + 120

    def fetch(url):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("public acceptance exceeded its two-minute deadline")
        request = urllib.request.Request(url, headers={"User-Agent": "LMM-Release-Acceptance/1.0", "Cache-Control": "no-cache", "Accept-Encoding": "identity"})
        with opener.open(request, timeout=min(15, remaining)) as response:
            body = response.read(MAX_BODY + 1)
            if len(body) > MAX_BODY:
                raise ValueError("public response exceeds the probe size limit")
            return Response(response.status, response.headers.get("Content-Type", ""), body,
                            dict(response.headers.items()))

    print(json.dumps(verify(fetch, args.expected_backend_version), sort_keys=True))


if __name__ == "__main__":
    main()
