# Existing-site search indexing overlay

This is an additive repair for the currently installed static frontend. It does not install the WIP application or replace backend, model, OAuth, billing, policy or WebSocket locations.

Capture the exact installed `index.html`, the entry JavaScript bundle referenced by that HTML, and its generated route catalog. The generator reconstructs the installed bundle's route graph and requires exact equality with the catalog before producing a usable route map. A source checkout alone is insufficient provenance. Build with:

```sh
python3 deployment/web-seo/build.py --index /private/capture/index.html \
  --routes /private/capture/routeTree.gen.ts --bundle /private/capture/installed-entry.js \
  --output /private/candidate \
  --remote-root /srv/lmm-api-frontend/seo/REVIEWED_ID
```

The output contains eight public HTML entry pages, metadata synchronized during browser navigation, a public sitemap, and a known-route map. Route matching uses the original request URI so `try_files` cannot lose its destination. Private workspaces are excluded from indexing. Public HTML is derived from the captured release and continues loading its original content-addressed JS/CSS.

Installation requires reviewing that host's current location inventory. Include `routes.conf` once in the HTTP context. Replace the old index location with `pages.conf`, the default SPA fallback with `fallback.conf`, and the client-side OAuth prefix with `oauth.conf`. Known-route guards return 404 for unknown paths; private pages receive `X-Robots-Tag: noindex, follow`. Do not edit the installed release in place. Preserve the complete prior configuration and record input and output hashes.

Run a real isolated Nginx check before activation and validate the complete installed configuration with `nginx -t`. Reload only Nginx. Verify public pages, canonical redirects, unknown paths, authentication/workspace navigation, immutable assets, backend routing and existing access-policy controls. Keep rollback files outside the live include glob. Reapply against a new capture when the frontend release or route catalog changes; this overlay is bound to its captured release.
