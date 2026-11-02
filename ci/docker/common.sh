#!/bin/sh
# Общая часть Linux-образов: rustup (stable) + bun (он же node для скриптов).
set -eu
BUN_VERSION="$1"
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
chmod -R a+rwX "$RUSTUP_HOME" "$CARGO_HOME"
curl -fsSL https://bun.sh/install | BUN_INSTALL=/usr/local/bun bash -s "bun-v${BUN_VERSION}"
ln -sf /usr/local/bun/bin/bun /usr/local/bin/bun
ln -sf /usr/local/bun/bin/bun /usr/local/bin/bunx
ln -sf /usr/local/bun/bin/bun /usr/local/bin/node
