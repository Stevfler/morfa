#!/bin/sh
# Загружает файлы из release/ в Generic Package Registry проекта и создаёт GitLab Release.
# Env: CI_* от GitLab; RELEASE_TOKEN (PAT со scope api) — опционально, если JOB-TOKEN'у не хватает прав.
set -eu

VERSION="${CI_COMMIT_TAG#v}"
PKG_URL="$CI_API_V4_URL/projects/$CI_PROJECT_ID/packages/generic/morfa/$VERSION"
if [ -n "${RELEASE_TOKEN:-}" ]; then AUTH="PRIVATE-TOKEN: $RELEASE_TOKEN"; else AUTH="JOB-TOKEN: $CI_JOB_TOKEN"; fi

LINKS="[]"
for f in release/*; do
  name=$(basename "$f")
  echo "Uploading $name"
  curl -fsS --retry 3 -H "$AUTH" --upload-file "$f" "$PKG_URL/$name" >/dev/null
  LINKS=$(echo "$LINKS" | jq --arg n "$name" --arg u "$PKG_URL/$name" \
    '. + [{name: $n, url: $u, direct_asset_path: ("/" + $n), link_type: "package"}]')
done

git fetch --tags --force >/dev/null 2>&1 || true
PREV=$(git describe --tags --abbrev=0 --match 'v*' "${CI_COMMIT_TAG}^" 2>/dev/null || true)
if [ -n "$PREV" ]; then RANGE="$PREV..$CI_COMMIT_TAG"; else RANGE="$CI_COMMIT_TAG"; fi
NOTES=$(git log "$RANGE" --no-merges --pretty=format:'- **%s** (%h)%n%w(0,2,2)%b' | sed '/^ *$/d')
[ -n "$NOTES" ] || NOTES="- Maintenance release"

DESC_FILE=$(mktemp)
{
  printf '## Изменения\n\n%s\n\n' "$NOTES"
  printf '## Установка\n\n'
  printf '| Система | Файл |\n|---|---|\n'
  printf '| Windows 10/11 | `*_x64-setup.exe` |\n'
  printf '| Ubuntu 22.04 / 24.04, Debian 12 | `*_<дистрибутив>_amd64.deb` |\n'
  printf '| Fedora | `*.x86_64.rpm` |\n'
  printf '| Arch Linux | `*.pkg.tar.zst` (`sudo pacman -U ...`) |\n'
  printf '| Любой Linux | `*.AppImage` |\n'
  [ -n "$PREV" ] && printf '\nСравнение: %s → %s\n' "$PREV" "$CI_COMMIT_TAG"
} > "$DESC_FILE"

jq -n --arg tag "$CI_COMMIT_TAG" --arg name "Morfa $CI_COMMIT_TAG" --rawfile desc "$DESC_FILE" --argjson links "$LINKS" \
  '{tag_name: $tag, name: $name, description: $desc, assets: {links: $links}}' > release.json

curl -fsS --retry 3 -H "$AUTH" -H "Content-Type: application/json" \
  --data @release.json "$CI_API_V4_URL/projects/$CI_PROJECT_ID/releases" >/dev/null
echo "Release $CI_COMMIT_TAG created"
