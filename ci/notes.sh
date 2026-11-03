#!/bin/sh
# Печатает в stdout описание релиза $CI_COMMIT_TAG (изменения с прошлого тега + таблица установки).
# Используется и для GitLab, и для GitHub релизов.
set -eu

git fetch --tags --force >/dev/null 2>&1 || true
PREV=$(git describe --tags --abbrev=0 --match 'v*' "${CI_COMMIT_TAG}^" 2>/dev/null || true)
if [ -n "$PREV" ]; then RANGE="$PREV..$CI_COMMIT_TAG"; else RANGE="$CI_COMMIT_TAG"; fi
NOTES=$(git log "$RANGE" --no-merges --pretty=format:'- **%s** (%h)%n%w(0,2,2)%b' | sed '/^ *$/d')
[ -n "$NOTES" ] || NOTES="- Maintenance release"

printf '## Изменения / Changes\n\n%s\n\n' "$NOTES"
printf '## Установка / Install\n\n'
printf '| Система / System | Файл / File |\n|---|---|\n'
printf '| Windows 10/11 | `*_x64-setup.exe` |\n'
printf '| Ubuntu 22.04 / 24.04, Debian 12 | `*_<distro>_amd64.deb` |\n'
printf '| Fedora | `*.x86_64.rpm` |\n'
printf '| Arch Linux | `*.pkg.tar.zst` (`sudo pacman -U ...`) |\n'
printf '| Linux (любой / any) | `*.AppImage` |\n'
if [ -n "$PREV" ]; then printf '\n%s → %s\n' "$PREV" "$CI_COMMIT_TAG"; fi
