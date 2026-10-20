#!/bin/sh
# Explicit operator action; never invoked by package installation or preparation.
set -eu
test "$(id -u)" = 0
test "$#" = 1
target=$1
case "$target" in /etc/nginx/lmm/modes/*.conf) ;; *) exit 2;; esac
test -f "$target" && test ! -L "$target"
test "$(stat -c %u "$target")" = 0
test -z "$(find "$target" -maxdepth 0 -perm /022 -print)"
link=/etc/nginx/lmm/site-mode.conf
test -L "$link" # Bootstrap integration is separately reviewed, never overwritten.
old=$(readlink "$link")
nginx -t
temporary=$(mktemp -d /etc/nginx/lmm/.mode.XXXXXXXX)
temp=$temporary/mode
trap 'rm -f "$temp"; rmdir "$temporary"' EXIT HUP INT TERM
ln -s "$target" "$temp"
mv -Tf "$temp" "$link"
if ! nginx -t; then
    ln -s "$old" "$temp"
    mv -Tf "$temp" "$link"
    nginx -t
    exit 1
fi
# A reload failure is uncertain: leave the selected files and inspect master/workers.
# Never restart Nginx or alter TLS as an automatic fallback.
systemctl reload nginx.service
printf '%s\n' 'Reload requested. Read back HTTPS status, Retry-After and worker state.'
