#!/usr/bin/env bash
# The release version rules, shared by CI's version-check (dev) and release-tag (main) jobs.
#   release-version.sh read <Cargo.toml>             prints the [workspace.package] version,
#                                                    exit 1 when it has none
#   release-version.sh dev-check <dev> <main>        exit 0 when dev > main (main may be empty)
#   release-version.sh tag-decision <ver> <tag_sha> <head_sha>
#                                                    prints CREATE or EXISTS, exit 1 on a moved tag
set -euo pipefail

semver_re='^[0-9]+\.[0-9]+\.[0-9]+$'

read_version() {
  local v
  v="$(awk '/^[ \t]*\[workspace\.package\]/{p=1;next} /^[ \t]*\[/{p=0}
            p && /^[ \t]*version[ \t]*=[ \t]*["\x27]/{sub(/^[^=]*=[ \t]*["\x27]/,""); sub(/["\x27].*/,""); print; exit}' "$1")"
  if [ -z "$v" ]; then
    echo "no [workspace.package] version in $1" >&2
    return 1
  fi
  echo "$v"
}

dev_check() {
  local dev="$1" main="$2"
  [[ "$dev" =~ $semver_re ]] || { echo "dev version '${dev}' is not a clean X.Y.Z"; return 1; }
  if [ -z "$main" ]; then
    echo "OK: dev ${dev}, main carries no version yet (first release)"
    return 0
  fi
  [[ "$main" =~ $semver_re ]] || { echo "main version '${main}' is not a clean X.Y.Z"; return 1; }
  local highest
  highest="$(printf '%s\n%s\n' "$dev" "$main" | sort -V | tail -1)"
  if [ "$dev" != "$main" ] && [ "$highest" = "$dev" ]; then
    echo "OK: dev ${dev} > main ${main}"
    return 0
  fi
  echo "dev ${dev} is not above main ${main}: bump [workspace.package] version in Cargo.toml"
  return 1
}

tag_decision() {
  local ver="$1" tag_sha="$2" head="$3"
  [[ "$ver" =~ $semver_re ]] || { echo "version '${ver}' is not a clean X.Y.Z" >&2; return 1; }
  if [ -z "$tag_sha" ]; then echo CREATE; return 0; fi
  if [ "$tag_sha" = "$head" ]; then echo EXISTS; return 0; fi
  echo "tag v${ver} already names ${tag_sha}, not ${head}: dev was not bumped before this release" >&2
  return 1
}

case "${1:-}" in
  read) read_version "$2" ;;
  dev-check) dev_check "$2" "${3:-}" ;;
  tag-decision) tag_decision "$2" "${3:-}" "$4" ;;
  *) echo "usage: $0 read|dev-check|tag-decision ..." >&2; exit 2 ;;
esac
