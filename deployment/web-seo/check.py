"""Exercise generated metadata, real Nginx route matching, and API precedence."""
import argparse
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

parser = argparse.ArgumentParser()
parser.add_argument('candidate', type=Path)
args = parser.parse_args()
candidate = args.candidate.resolve()
pages = (candidate / 'pages.conf').read_text()
remote_root = re.search(r'alias ([^;]+)/private.html;', pages)[1]
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None
opener = urllib.request.build_opener(NoRedirect)
def get(path):
    try:
        response = opener.open(f'http://127.0.0.1:{port}{path}', timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()
with tempfile.TemporaryDirectory(prefix='lmm-seo-check-') as temp:
    root = Path(temp)
    directives = [f'{kind}_temp_path {root}/{kind};' for kind in ['client_body','proxy','fastcgi','uwsgi','scgi']]
    conf = root / 'nginx.conf'
    conf.write_text(f'pid {root}/nginx.pid; error_log {root}/error.log; daemon off; master_process off; events {{}}\n'
                    'http { include /etc/nginx/mime.types; default_type text/plain; access_log off; types_hash_max_size 4096;\n'
                    + '\n'.join(directives) + '\n' + (candidate / 'routes.conf').read_text()
                    + f'\nserver {{ listen 127.0.0.1:{port}; root {candidate};\n'
                    + pages.replace(remote_root, str(candidate))
                    + (candidate / 'oauth.conf').read_text().replace(remote_root, str(candidate))
                    + '\nlocation ^~ /api/ { return 204; }\nlocation ^~ /v1/ { return 202; }\n'
                    + (candidate / 'fallback.conf').read_text().replace(remote_root, str(candidate)) + '\n}}\n')
    process = subprocess.Popen(['nginx', '-p', temp, '-c', str(conf)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        for attempt in range(40):
            try:
                get('/')
                break
            except urllib.error.URLError:
                if process.poll() is not None:
                    raise RuntimeError(process.communicate()[1].decode())
                time.sleep(.05)
        else:
            raise RuntimeError('Nginx startup timed out')
        for path in ['/', '/pricing?utm_source=test', '/guide', '/about']:
            status, headers, body = get(path)
            assert status == 200 and b'rel="canonical"' in body and b'<h1>' in body, (path, status, (root / 'error.log').read_text())
            assert re.search(rb'<div\b[^>]*\bid="root"[^>]*>\s*</div>', body), 'Installed app needs an empty React root'
            assert b'<main id="lmm-seo-content">' in body
            assert 'X-Robots-Tag' not in headers, path
            assert headers.get_content_type() == 'text/html', path
        for path in ['/sign-in?redirect=/dashboard', '/dashboard', '/dashboard/overview', '/challenges/1', '/oauth/google', '/index.html']:
            status, headers, body = get(path)
            assert status == 200 and headers['X-Robots-Tag'] == 'noindex, follow', path
            assert b'rel="canonical"' not in body, path
            assert headers.get_content_type() == 'text/html', path
        for path in ['/v111', '/does-not-exist', '/oauth/a/b/c', '/%EF%BC%8C%E5%B9%B6%E4%BD%BF%E7%94%A8%E5%90%88%E9%80%82%E7%9A%84']:
            assert get(path)[0] == 404, path
        status, headers, body = get('/guide/?ref=test')
        assert status == 308 and headers['Location'] == 'https://api.lmm.best/guide?ref=test'
        assert get('/api/status')[0] == 204
        assert get('/v1/models')[0] == 202
        assert get('/sitemap.xml')[0] == 200
        print('Public metadata, private routes, query strings, 404s, redirects and API precedence passed')
    finally:
        if process.poll() is None:
            process.terminate()
        process.communicate(timeout=5)
