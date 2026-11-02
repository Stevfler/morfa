#!/usr/bin/env sh
# Кладёт libvips (официальная сборка build-win64-mxe, вариант web-static) в src-tauri/vendor/vips-win,
# откуда ci/tauri.vips-win.json забирает её в ресурсы установщика (<resource_dir>/bin, см. sidecar.rs).
set -eu
VER=8.18.7
SHA=a736af61d3b46499760efa606e469cb14e060445b1f6fd36a1fa2e75009e1197
URL="https://github.com/libvips/build-win64-mxe/releases/download/v$VER/vips-dev-x64-web-$VER-static.zip"
OUT=src-tauri/vendor/vips-win
TMP=$(mktemp -d)
curl -fsSL -o "$TMP/v.zip" "$URL"
echo "$SHA  $TMP/v.zip" | sha256sum -c -
rm -rf "$OUT" && mkdir -p "$OUT"
unzip -q -j "$TMP/v.zip" '*/bin/vips.exe' '*/bin/vipsheader.exe' '*/bin/libvips-42.dll' -d "$OUT"
rm -rf "$TMP"
ls -la "$OUT"
