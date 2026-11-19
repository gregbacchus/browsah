#!/usr/bin/env bash
# Embedded by the CLI and used to select the installed CEF before startup.
set -euo pipefail

case "${XDG_DATA_HOME:-}" in
  /*) data_home="$XDG_DATA_HOME" ;;
  *) data_home="$HOME/.local/share" ;;
esac
system_prefix="${BROWSAH_CODEC_SYSTEM:-}"
if [[ -n "$system_prefix" ]]; then
  base="$system_prefix/codec-runtime"
  [[ ! -L "$base" ]] || { echo 'codecs: system runtime directory must not be a symlink' >&2; exit 1; }
else
  base="$(realpath -m "$data_home/browsah/codec-runtime")"
fi
archive_name='cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_linux64_minimal.zip'
digest='027473508ec25227972a8bc9182b15723f98cfa2e301746f8e2224c6a58517d5'
api_hash='6dc09eb2834fe5b61d7d8c42bbb8d8c27a2bb1b6'
url='https://github.com/tobagin/karere/releases/download/cef-156.0.26-proprietary-codecs-2/cef_binary_156.0.26%2Bg8ea0c73%2Bchromium-156.0.8078.88_linux64_minimal.zip'

fail() { echo "codecs: $*" >&2; exit 1; }
hash_api() {
  LD_LIBRARY_PATH="$1${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" python3 - "$1/libcef.so" <<'PY'
import ctypes, sys
lib = ctypes.CDLL(sys.argv[1])
lib.cef_api_hash.argtypes = (ctypes.c_int, ctypes.c_int)
lib.cef_api_hash.restype = ctypes.c_char_p
value = lib.cef_api_hash(15600, 0)
if not value:
    raise SystemExit('CEF API 15600 is unavailable')
print(value.decode('ascii'))
lib.cef_version_info.argtypes = (ctypes.c_int,)
lib.cef_version_info.restype = ctypes.c_int
print('.'.join(str(lib.cef_version_info(i)) for i in (0, 1, 2, 4, 5, 6, 7)))
PY
}
check_api() {
  local actual expected
  expected="$(printf '%s\n%s' "$api_hash" "$2")"
  actual="$(hash_api "$1")" || fail "cannot read the CEF API hash in $1"
  [[ "$actual" == "$expected" ]] || fail "CEF in $1 has an untested API or version; cannot use this runtime"
}
action="${1:-help}"
[[ $# -eq 0 ]] || shift
if [[ "$action" == install || "$action" == uninstall ]]; then
  if [[ -n "$system_prefix" ]]; then
    [[ $EUID -eq 0 ]] || fail "use sudo with the full path to the system launcher to $action system codecs"
    umask 022
  else
    [[ $EUID -ne 0 ]] || fail 'run without sudo, or use the root-owned system launcher for system codecs'
  fi
fi
case "$action" in
  install)
    [[ "$(uname -m)" == x86_64 ]] || fail 'this tested runtime currently supports x86_64 only'
    for tool in python3 sha256sum unzip; do command -v "$tool" >/dev/null || fail "install $tool first"; done
    source_dir="${system_prefix:-${BROWSAH_CODEC_SOURCE:?missing browsah installation}}"
    source_launcher="${BROWSAH_CODEC_LAUNCHER:?missing browsah launcher}"
    [[ -f "$source_dir/browsah" && -f "$source_dir/libcef.so" ]] || fail "no browsah and libcef.so in $source_dir"
    check_api "$source_dir" '156.0.28.156.0.8078.88'
    mkdir -p "$base"
    stage="$(mktemp -d "$base/.install.XXXXXXXX")"
    trap 'rm -rf "$stage"' EXIT
    if [[ "${1:-}" == --archive && $# -eq 2 ]]; then
      cp -- "$2" "$stage/$archive_name"
    elif [[ $# -eq 0 ]]; then
      command -v curl >/dev/null || fail 'install curl first'
      echo 'Downloading the codec-enabled CEF runtime (about 190 MB)...'
      curl -fL --retry 2 -o "$stage/$archive_name" "$url"
    else
      fail 'usage: browsah codecs install [--archive FILE]'
    fi
    printf '%s  %s\n' "$digest" "$stage/$archive_name" | sha256sum -c -
    unzip -q "$stage/$archive_name" -d "$stage/unpacked"
    unpacked="$stage/unpacked/${archive_name%.zip}"
    [[ -d "$unpacked/Release" && -d "$unpacked/Resources" ]] || fail 'archive has an unexpected layout'
    mkdir "$stage/runtime"
    cp -a --no-preserve=ownership "$unpacked/Release/." "$stage/runtime/"
    cp -a --no-preserve=ownership "$unpacked/Resources/." "$stage/runtime/"
    # Browsah does not use the Chromium setuid sandbox helper.
    rm -f -- "$stage/runtime/chrome-sandbox"
    cp -a "$unpacked/LICENSE.txt" "$unpacked/CREDITS.html" "$stage/runtime/"
    cp "$source_dir/browsah" "$stage/runtime/browsah"
    check_api "$stage/runtime" '156.0.26.156.0.8078.88'
    printf '%s\n' "$source_dir" > "$stage/runtime/source-dir"
    printf '%s\n' "$source_launcher" > "$stage/runtime/source-launcher"
    sha256sum "$source_dir/browsah" | cut -d' ' -f1 > "$stage/runtime/source-sha256"
    if [[ -n "$system_prefix" ]]; then
      chmod -R a+rX,go-w,ug-s "$stage/runtime"
    fi
    # Stage a new generation before changing the active link; do not delete an old runtime
    # that may still be used by a running profile.
    generation="$(mktemp -d "$base/cef-156.0.26.XXXXXXXX")"
    rmdir "$generation"
    mv "$stage/runtime" "$generation"
    ln -s "${generation##*/}" "$stage/current"
    mv -Tf "$stage/current" "$base/current"
    echo 'Installed. New browsah launches will use the codec-enabled runtime.'
    echo 'Close an existing window of the same profile first so it uses this runtime.'
    ;;
  uninstall)
    [[ $# -eq 0 ]] || fail 'usage: browsah codecs uninstall'
    # A running browser and its CEF subprocesses may still load files from their
    # generation. Do not remove those files until all codec-enabled processes exit.
    for proc in /proc/[0-9]*/exe; do
      exe="$(readlink "$proc" 2>/dev/null || true)"
      case "$exe" in "$base"/cef-156.0.26.*/browsah) fail 'close all codec-enabled browsah windows before uninstalling' ;; esac
    done
    if [[ -e "$base/current" && ! -L "$base/current" ]]; then
      fail "$base/current is not a symlink; refusing to remove it"
    fi
    rm -f -- "$base/current" "$base/launcher"
    for generation in "$base"/cef-156.0.26.*; do
      [[ -d "$generation" && ! -L "$generation" ]] || continue
      rm -rf -- "$generation"
    done
    rmdir "$base" 2>/dev/null || true
    echo 'Uninstalled. New browsah launches will use the bundled CEF runtime.'
    ;;
  status|run)
    runtime="$(readlink -f "$base/current")" || fail 'not installed; run: browsah codecs install'
    [[ -f "$runtime/browsah" && -f "$runtime/source-dir" && -f "$runtime/source-launcher" && -f "$runtime/source-sha256" ]] || fail 'not installed; run: browsah codecs install'
    IFS= read -r source_dir < "$runtime/source-dir"
    IFS= read -r source_launcher < "$runtime/source-launcher"
    IFS= read -r saved_hash < "$runtime/source-sha256"
    caller="${BROWSAH_CODEC_CALLER:-${system_prefix:-${BROWSAH_CODEC_SOURCE:-}}/browsah}"
    [[ -f "$caller" && "$(sha256sum "$caller" | cut -d' ' -f1)" == "$saved_hash" ]] || fail 'this browsah installation differs from the codec runtime; run: browsah codecs install'
    [[ -f "$source_dir/browsah" && -f "$source_dir/libcef.so" ]] || fail 'the original browsah installation is missing; reinstall the runtime'
    current_hash="$(sha256sum "$source_dir/browsah" | cut -d' ' -f1)"
    [[ "$saved_hash" == "$current_hash" ]] || fail 'browsah changed since installation; run: browsah codecs install'
    [[ "$(sha256sum "$runtime/browsah" | cut -d' ' -f1)" == "$saved_hash" ]] || fail 'the installed browsah copy changed; reinstall the runtime'
    check_api "$source_dir" '156.0.28.156.0.8078.88'
    check_api "$runtime" '156.0.26.156.0.8078.88'
    if [[ "$action" == status ]]; then
      echo "Codec runtime installed: $runtime"
      echo 'CEF 156.0.26, Chromium 156.0.8078.88; compatible API 15600.'
      exit 0
    fi
    export BROWSAH_LAUNCHER="$source_launcher" BROWSAH_LIBS="$source_dir" BROWSAH_CODEC_ACTIVE=1
    export LD_LIBRARY_PATH="$runtime${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    exec "$runtime/browsah" "$@"
    ;;
  help|--help|-h)
    echo 'Usage: browsah codecs install [--archive FILE] | status | uninstall'
    echo 'Downloads a separately installed H.264/AAC runtime for personal use (Linux x86_64).'
    ;;
  *) fail 'usage: browsah codecs install [--archive FILE] | status | uninstall' ;;
esac
