#!/bin/sh
# Drop Linux app-menu entries that launch a development build of this repo.
# A development build loads the dev server of its app (Vite for the desktop,
# Metro for the phone) and reports that the server is not running once that
# server has stopped.
# Run this before installing a release build. Linux only.

set -eu

if [ "$(uname -s)" != "Linux" ]; then
  echo "linux-prepare-install: Linux only" >&2
  exit 1
fi

apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
removed=0
warned=""

is_dev_entry() {
  exec_line=$1
  case "$exec_line" in
    *"/target/debug/"*)
      return 0
      ;;
  esac
  return 1
}

is_dev_server() {
  exec_line=$1
  case "$exec_line" in
    *expo*start* | *react-native*start* | *metro*)
      return 0
      ;;
  esac
  return 1
}

resolve_exec() {
  raw=$1
  raw=${raw#Exec=}
  # shellcheck disable=SC2086
  set -- $raw
  cmd=${1:-}
  cmd=${cmd#\"}
  cmd=${cmd%\"}
  if [ -z "$cmd" ]; then
    return 1
  fi
  case "$cmd" in
    env)
      shift
      while [ $# -gt 0 ]; do
        case "$1" in
          *=*) shift ;;
          *)
            cmd=$1
            break
            ;;
        esac
      done
      ;;
  esac
  if [ -x "$cmd" ]; then
    printf '%s\n' "$cmd"
    return 0
  fi
  command -v "$cmd" 2>/dev/null || return 1
}

warn_unstripped() {
  bin=$1
  [ -f "$bin" ] || return 0
  if ! command -v file >/dev/null 2>&1; then
    return 0
  fi
  if file -b "$bin" 2>/dev/null | grep -q 'not stripped'; then
    echo "warning: $bin looks like a debug build (not stripped)." >&2
    echo "A debug build expects the dev server of its app. Install a release bundle instead." >&2
  fi
}

if [ -d "$apps" ]; then
  for file in "$apps"/*.desktop; do
    [ -f "$file" ] || continue
    base=$(basename "$file")
    case "$base" in
      *k-virtual-monitor* | *kvm*) ;;
      *) continue ;;
    esac
    exec_line=$(grep -m1 -E '^Exec=' "$file" || true)
    [ -n "$exec_line" ] || continue
    if is_dev_entry "$exec_line" || is_dev_server "$exec_line"; then
      rm -f "$file"
      echo "removed $file"
      removed=$((removed + 1))
      continue
    fi
    bin=$(resolve_exec "$exec_line" || true)
    if [ -n "$bin" ]; then
      case " $warned " in
        *" $bin "*) ;;
        *)
          warn_unstripped "$bin"
          warned="$warned $bin"
          ;;
      esac
    fi
  done
fi

if command -v update-desktop-database >/dev/null 2>&1 && [ -d "$apps" ]; then
  update-desktop-database "$apps" || true
fi

echo "linux-prepare-install: removed $removed desktop entries"
echo "Install the release bundle next. Do not copy src-tauri/target/debug onto the menu launcher."
