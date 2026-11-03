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

DESC_FILE=$(mktemp)
sh ci/notes.sh > "$DESC_FILE"

jq -n --arg tag "$CI_COMMIT_TAG" --arg name "Morfa $CI_COMMIT_TAG" --rawfile desc "$DESC_FILE" --argjson links "$LINKS" \
  '{tag_name: $tag, name: $name, description: $desc, assets: {links: $links}}' > release.json

curl -fsS --retry 3 -H "$AUTH" -H "Content-Type: application/json" \
  --data @release.json "$CI_API_V4_URL/projects/$CI_PROJECT_ID/releases" >/dev/null
echo "Release $CI_COMMIT_TAG created"
