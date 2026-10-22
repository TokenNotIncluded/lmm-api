"""Build an additive SEO overlay for an explicitly captured frontend release.

No API locations or compiled application assets are generated or replaced.
The caller supplies that release's HTML and its matching route catalog.
"""
import argparse
import hashlib
import html
import json
from pathlib import Path
import re
from urllib.parse import urlsplit

PAGES = {
    '/': ('AI model access and collaboration', 'Connect AI models, tools and applications with LMM Forge. Explore model access, developer guides and open-source collaboration.'),
    '/guide': ('Developer guide', 'Get started with LMM Forge model access, client configuration and developer tools.'),
    '/security': ('Security', 'Read about LMM Forge account security and responsible use of credentials and model access.'),
    '/privacy-policy': ('Privacy policy', 'Read the LMM Forge privacy policy and learn how account and service information is handled.'),
    '/user-agreement': ('User agreement', 'Read the LMM Forge user agreement, account rules and service terms.'),
    '/refund-policy': ('Refund policy', 'Read the LMM Forge refund policy and the rules that apply to orders and requests.'),
}

def route_patterns(source, bundle):
    section = source.split('  fullPaths:', 1)[1].split('  fileRoutesByTo:', 1)[0]
    routes = sorted(set(re.findall(r"\|\s*'([^']+)'", section)))
    if '/' not in routes or len(routes) < 10:
        raise ValueError('Expected a generated TanStack fullPaths catalog')
    nodes = {name: (path, parent) for name, path, parent in re.findall(
        r'([\w$]+)=[\w$.]+\.update\(\{id:"[^"]+"(?:,path:"([^"]*)")?,getParentRoute:\(\)=>([\w$]+)\}\)', bundle)}
    roots = {parent for _, parent in nodes.values() if parent not in nodes}
    if len(roots) != 1:
        raise ValueError('Cannot establish the installed bundle route root')
    def full_path(name, visiting=frozenset()):
        if name not in nodes:
            return ''
        if name in visiting:
            raise ValueError('Cyclic installed route graph')
        path, parent = nodes[name]
        base = full_path(parent, visiting | {name})
        return base.rstrip('/') + '/' + path.lstrip('/') if path else base
    installed = {full_path(name) for name, (path, _) in nodes.items() if path}
    if installed != set(routes):
        raise ValueError(f'Route catalog differs from installed bundle: missing={installed-set(routes)}, extra={set(routes)-installed}')
    patterns = []
    for route in routes:
        escaped = re.escape(route.rstrip('/'))
        escaped = re.sub(r'\\\$[A-Za-z_][A-Za-z_0-9]*', '[^/?]+', escaped)
        patterns.append('^' + (escaped + '/?' if route != '/' else '/') + r'(?:\?.*)?$')
    return patterns

def metadata(template, path, origin):
    title, description = PAGES[path]
    title += ' · LMM Forge'
    head = template.split('</head>', 1)[0]
    head = re.sub(r'<title\b[^>]*>.*?</title>', '', head, flags=re.S | re.I)
    head = re.sub(r'<meta\b(?=[^>]*(?:name=[\"\'](?:title|description|robots)[\"\']|property=[\"\']og:[^\"\']+[\"\']))[^>]*>', '', head, flags=re.S | re.I)
    head = re.sub(r'<link\b(?=[^>]*rel=[\"\']canonical[\"\'])[^>]*>', '', head, flags=re.S | re.I)
    esc = html.escape
    canonical = origin + path
    head += (f'<title>{esc(title)}</title><meta name="description" content="{esc(description, quote=True)}">'
             '<meta name="robots" content="index, follow">'
             f'<link rel="canonical" href="{canonical}">'
             f'<meta property="og:type" content="website"><meta property="og:title" content="{esc(title, quote=True)}">'
             f'<meta property="og:description" content="{esc(description, quote=True)}"><meta property="og:url" content="{canonical}">')
    body = template.split('</head>', 1)[1]
    nav = ' · '.join(f'<a href="{p}">{html.escape(value[0])}</a>' for p, value in PAGES.items())
    body, count = re.subn(r'(<div\b[^>]*\bid="root"[^>]*>)\s*</div>',
                         lambda match: match[1] + f'</div><main id="lmm-seo-content"><h1>{esc(title)}</h1><p>{esc(description)}</p><nav>{nav}</nav></main>', body, count=1)
    if count != 1:
        raise ValueError('Expected an empty React root')
    return head + '</head>' + body

def build(index, routes, bundle, output, remote_root, origin='https://api.lmm.best'):
    url = urlsplit(origin)
    if url.scheme != 'https' or not url.netloc or url.path or url.query or url.fragment or url.username:
        raise ValueError('Origin must be a credential-free HTTPS origin')
    if not re.fullmatch(r'/[A-Za-z0-9/_.-]+', remote_root):
        raise ValueError('Expected an absolute, plain remote directory')
    output.mkdir(parents=True, exist_ok=False)
    template = index.replace('</head>', '<script defer src="/site-seo.js"></script></head>')
    (output / 'private.html').write_text(template.replace('</head>', '<meta name="robots" content="noindex, follow"></head>'))
    patterns = route_patterns(routes, bundle)
    (output / 'routes.conf').write_text('map $request_uri $lmm_seo_known_route {\n    default 0;\n' + ''.join(f'    "~{p}" 1;\n' for p in patterns) + '}\n')
    snippets = []
    snippets.append(f'location = /index.html {{\n    alias {remote_root}/private.html;\n    default_type text/html;\n    add_header X-Robots-Tag "noindex, follow" always;\n    add_header Cache-Control "no-cache, must-revalidate" always;\n}}\n')
    for i, path in enumerate(PAGES):
        filename = f'page-{i}.html'
        (output / filename).write_text(metadata(template, path, origin))
        serve = f'root {remote_root};\n    try_files /{filename} =404;' if path == '/' else f'alias {remote_root}/{filename};'
        snippets.append(f'location = {path} {{\n    {serve}\n    default_type text/html;\n    add_header Cache-Control "no-cache, must-revalidate" always;\n}}\n')
        if path != '/':
            snippets.append(f'location = {path}/ {{ return 308 {origin}{path}$is_args$args; }}\n')
    for path, filename in [('/robots.txt', 'robots.txt'), ('/sitemap.xml', 'sitemap.xml'), ('/site-seo.js', 'site-seo.js')]:
        snippets.append(f'location = {path} {{\n    auth_request off;\n    alias {remote_root}/{filename};\n    add_header Cache-Control "no-cache, must-revalidate" always;\n}}\n')
    (output / 'pages.conf').write_text('\n'.join(snippets))
    (output / 'fallback.conf').write_text('location / {\n    try_files $uri $uri/ @lmm_seo_spa;\n    add_header Cache-Control "no-cache, must-revalidate" always;\n}\n'
        'location @lmm_seo_spa {\n    if ($lmm_seo_known_route = 0) { return 404; }\n'
        f'    root {remote_root};\n    try_files /private.html =404;\n'
        '    add_header X-Robots-Tag "noindex, follow" always;\n    add_header Cache-Control "no-cache, must-revalidate" always;\n}\n')
    (output / 'oauth.conf').write_text('location ^~ /oauth/ {\n    if ($lmm_seo_known_route = 0) { return 404; }\n'
        f'    root {remote_root};\n    try_files /private.html =404;\n'
        '    add_header X-Robots-Tag "noindex, follow" always;\n    add_header Cache-Control "no-cache, must-revalidate" always;\n}\n')
    (output / 'robots.txt').write_text(f'User-agent: *\nAllow: /\n\nSitemap: {origin}/sitemap.xml\n')
    (output / 'sitemap.xml').write_text('<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n' + ''.join(f'  <url><loc>{origin}{p}</loc></url>\n' for p in PAGES) + '</urlset>\n')
    script = (Path(__file__).with_name('metadata.js')).read_text()
    (output / 'site-seo.js').write_text(script.replace('/* PAGE_CATALOG */ {}', json.dumps(PAGES, ensure_ascii=True)).replace('/* PUBLIC_ORIGIN */ ""', json.dumps(origin)))
    (output / 'manifest.json').write_text(json.dumps({'original_html_sha256': hashlib.sha256(index.encode()).hexdigest(), 'route_catalog_sha256': hashlib.sha256(routes.encode()).hexdigest(), 'installed_bundle_sha256': hashlib.sha256(bundle.encode()).hexdigest(), 'verified_route_count': len(patterns), 'origin': origin, 'public_pages': list(PAGES)}, indent=2) + '\n')

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--index', type=Path, required=True)
    parser.add_argument('--routes', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--remote-root', required=True)
    args = parser.parse_args()
    build(args.index.read_text(), args.routes.read_text(), args.bundle.read_text(), args.output, args.remote_root)
