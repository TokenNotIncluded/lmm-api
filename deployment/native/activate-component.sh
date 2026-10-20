#!/bin/sh
# Optional /opt store only; do not use for the AUR/pacman /usr/bin deployment.
# Explicit local host operation, not an installer hook. Keep maintenance enabled
# until the independent operator has verified actual business readback.
set -eu
test "$(id -u)" = 0
test "$#" = 11
component=$1
candidate=$2
release_sha=$3
review=$4
review_sha=$5
host=$6
database_sha=$7
mode=$8
previous=$9
shift 9
previous_sha=$1
tool_directory=$2
test "$(hostname)" = "$host"
case "$component" in core|extensions|web) ;; *) exit 2;; esac
case "$candidate" in /opt/lmm/releases/*) ;; *) exit 2;; esac
test "$(basename "$candidate")" = "$release_sha"
# All deployment tools, releases and reviews must be root-owned, not writable
# by either service or Nginx. Provision them before running this entry point.
for directory in /opt/lmm /opt/lmm/releases /opt/lmm/current "$candidate" "$tool_directory"; do
    test -d "$directory" && test ! -L "$directory"
    test "$(stat -c %u "$directory")" = 0
    test -z "$(find "$directory" -maxdepth 0 -perm /022 -print)"
done
test -z "$(find "$candidate" "$tool_directory" -not -user root -print -quit)"
exec 9>/run/lock/lmm-native-update.lock
flock -n 9
python3 "$tool_directory/prepare.py" gate --mode "$mode" \
    --release-dir "$candidate" --release-sha256 "$release_sha" \
    --evidence "$review" --evidence-sha256 "$review_sha" \
    --host "$host" --database-identity-sha256 "$database_sha" \
    --previous-dir "$previous" --previous-sha256 "$previous_sha"
test -d "$candidate/$component"
link=/opt/lmm/current/$component
if test "$mode" = update; then
    test -L "$link"
    test "$(readlink -f "$link")" = "$(realpath "$previous/$component")"
else
    test ! -e "$link" && test ! -L "$link"
fi
if test "$component" != web; then
    systemctl stop "lmm-$component.service"
    # Refuse switching while an old process is still draining or failed to stop.
    state=$(systemctl show "lmm-$component.service" -p ActiveState --value)
    test "$state" = inactive
    test "$(systemctl show "lmm-$component.service" -p MainPID --value)" = 0
fi
temporary=$(mktemp -d /opt/lmm/current/.switch.XXXXXXXX)
trap 'rm -f "$temporary/component"; rmdir "$temporary"' EXIT HUP INT TERM
ln -s "$candidate/$component" "$temporary/component"
mv -Tf "$temporary/component" "$link"
if test "$component" != web; then
    systemctl start "lmm-$component.service"
fi
printf '%s\n' 'Candidate selected. Keep maintenance until process, readiness and business checks pass.'
# No database initialization, migration, deletion, force-kill or software downgrade.
