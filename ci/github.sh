#!/bin/sh
# Публичное зеркало на GitHub.
#   ci/github.sh mirror  — пушит текущую ветку (main) или тег в $GITHUB_REPO.
#   ci/github.sh release — создаёт GitHub Release для $CI_COMMIT_TAG и загружает файлы из release/.
# Env: GITHUB_TOKEN — fine-grained PAT только на этот репозиторий, права Contents: Read and write
#      (masked + protected переменная в GitLab); GITHUB_REPO — owner/name (по умолчанию Stevfler/morfa).
set -eu

REPO="${GITHUB_REPO:-Stevfler/morfa}"
API="https://api.github.com/repos/$REPO"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is not set}"

gh_api() {
  curl -fsS --retry 3 -H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" "$@"
}

case "${1:-}" in
mirror)
  # Токен только в URL этой команды: не попадает в .git/config и в лог (переменная masked).
  URL="https://x-access-token:$GITHUB_TOKEN@github.com/$REPO.git"
  if [ -n "${CI_COMMIT_TAG:-}" ]; then
    git push "$URL" "refs/tags/$CI_COMMIT_TAG:refs/tags/$CI_COMMIT_TAG"
  else
    # GitLab — источник истины, поэтому GitHub приводится к нему, даже если там что-то правили руками.
    git push --force "$URL" "$CI_COMMIT_SHA:refs/heads/$CI_COMMIT_BRANCH"
  fi
  ;;
release)
  NOTES=$(mktemp)
  sh ci/notes.sh > "$NOTES"
  jq -n --arg tag "$CI_COMMIT_TAG" --arg name "Morfa $CI_COMMIT_TAG" --rawfile body "$NOTES" \
    '{tag_name: $tag, name: $name, body: $body, draft: false, prerelease: false}' > gh-release.json
  # Повторный запуск job'а: релиз уже есть — берём его, загружаем недостающие файлы.
  ID=$(gh_api "$API/releases/tags/$CI_COMMIT_TAG" 2>/dev/null | jq -r '.id // empty' || true)
  [ -n "$ID" ] || ID=$(gh_api -X POST --data @gh-release.json "$API/releases" | jq -r .id)
  HAVE=$(gh_api "$API/releases/$ID/assets?per_page=100" | jq -r '.[].name')
  for f in release/*; do
    name=$(basename "$f")
    if printf '%s\n' "$HAVE" | grep -qxF "$name"; then echo "Skip $name (already uploaded)"; continue; fi
    echo "Uploading $name"
    gh_api -X POST -H "Content-Type: application/octet-stream" --data-binary @"$f" \
      "https://uploads.github.com/repos/$REPO/releases/$ID/assets?name=$(printf %s "$name" | jq -sRr @uri)" >/dev/null
  done
  echo "GitHub release $CI_COMMIT_TAG: https://github.com/$REPO/releases/tag/$CI_COMMIT_TAG"
  ;;
*)
  echo "usage: $0 mirror|release" >&2
  exit 2
  ;;
esac
