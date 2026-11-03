#!/bin/sh
# Сборка Linux-пакета в текущем контейнере.
# Env: DISTRO (метка в имени файла), BUNDLES (deb | rpm | appimage | arch), CARGO_TARGET_DIR.
set -eu

VERSION=$(jq -r .version package.json)
OUT="$CI_PROJECT_DIR/release"
mkdir -p "$OUT"

TAURI_BUNDLES="$BUNDLES"
[ "$BUNDLES" = "arch" ] && TAURI_BUNDLES=deb

bun install --frozen-lockfile
# AppImage обновляется встроенным апдейтером: собираем его с подписью (.sig рядом с файлом).
# Нужны TAURI_SIGNING_PRIVATE_KEY и TAURI_SIGNING_PRIVATE_KEY_PASSWORD (переменные GitLab).
if [ "$BUNDLES" = "appimage" ]; then
  bunx tauri build --bundles "$TAURI_BUNDLES" --config ci/updater.conf.json
else
  bunx tauri build --bundles "$TAURI_BUNDLES"
fi

BUNDLE_DIR="$CARGO_TARGET_DIR/release/bundle"
case "$BUNDLES" in
  deb)
    cp "$(ls "$BUNDLE_DIR"/deb/*_"$VERSION"_*.deb | head -n1)" "$OUT/Morfa_${VERSION}_${DISTRO}_amd64.deb"
    ;;
  rpm)
    cp "$(ls "$BUNDLE_DIR"/rpm/*-"$VERSION"-*.rpm | head -n1)" "$OUT/Morfa_${VERSION}_${DISTRO}.x86_64.rpm"
    ;;
  appimage)
    APPIMAGE=$(ls "$BUNDLE_DIR"/appimage/*_"$VERSION"_*.AppImage | head -n1)
    cp "$APPIMAGE" "$OUT/Morfa_${VERSION}_amd64.AppImage"
    # Подпись не зависит от имени файла, поэтому переименовывать её вместе с AppImage безопасно.
    cp "$APPIMAGE.sig" "$OUT/Morfa_${VERSION}_amd64.AppImage.sig"
    ;;
  arch)
    WORK=$(mktemp -d)
    cp ci/arch/PKGBUILD "$WORK/"
    cp "$(ls "$BUNDLE_DIR"/deb/*_"$VERSION"_*.deb | head -n1)" "$WORK/morfa.deb"
    chown -R builder "$WORK"
    su builder -c "cd '$WORK' && MORFA_VERSION='$VERSION' DEB_PATH='$WORK/morfa.deb' PKGDEST='$WORK/out' makepkg -f --nodeps"
    cp "$WORK"/out/morfa-"$VERSION"-*.pkg.tar.zst "$OUT/"
    ;;
  *)
    echo "Unknown BUNDLES=$BUNDLES"
    exit 1
    ;;
esac

ls -la "$OUT"
