#!/usr/bin/env bash
# Tests for scripts/release-version.sh (run by CI's version-rules job).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
rv="$here/release-version.sh"
fails=0
expect() { # expect <want-exit> <want-stdout-or-empty> <args...>
  local want_rc="$1" want_out="$2"; shift 2
  local out rc=0
  out="$("$rv" "$@")" || rc=$?
  if [ "$rc" != "$want_rc" ] || { [ -n "$want_out" ] && [ "$out" != "$want_out" ]; }; then
    echo "FAIL: $* -> rc=$rc out='$out' (want rc=$want_rc out='$want_out')"; fails=$((fails+1))
  else
    echo "ok: $*"
  fi
}
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "1.2.3"\n\n[x]\nversion = "9.9.9"\n' > "$tmp/Cargo.toml"
expect 0 "1.2.3" read "$tmp/Cargo.toml"
expect 0 "" dev-check 0.1.0 ""
expect 0 "" dev-check 0.1.1 0.1.0
expect 0 "" dev-check 0.10.0 0.9.0
expect 1 "" dev-check 0.1.0 0.1.0
expect 1 "" dev-check 0.1.0 0.2.0
expect 1 "" dev-check 0.1.0-rc1 0.0.9
expect 0 "CREATE" tag-decision 0.1.0 "" abc
expect 0 "EXISTS" tag-decision 0.1.0 abc abc
expect 1 "" tag-decision 0.1.0 def abc
[ "$fails" -eq 0 ] || { echo "$fails failure(s)"; exit 1; }
echo "all release-version tests passed"
