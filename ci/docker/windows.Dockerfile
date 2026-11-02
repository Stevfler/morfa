# Кросс-сборка Morfa под Windows x64 на Linux-раннере (cargo-xwin + NSIS).
# trixie: NSIS 3.11 — шаблону tauri-cli >= 2.12 нужен Include/Win/RestartManager.nsh (нет в 3.08 из bookworm).
FROM rust:1-trixie

ARG BUN_VERSION=1.4.4
ARG CARGO_XWIN_VERSION=0.19.2

RUN apt-get update && apt-get install -y --no-install-recommends \
      nsis clang lld llvm jq curl unzip ca-certificates git \
      # tauri-cli на Linux ищет appindicator через pkg-config даже при сборке под Windows
      pkg-config libayatana-appindicator3-dev \
  && rm -rf /var/lib/apt/lists/* \
  # cargo-xwin ищет clang-cl / lld-link / llvm-lib без суффикса версии
  && for t in clang-cl lld-link llvm-lib llvm-rc llvm-ar; do \
       [ -e /usr/bin/$t ] || ln -s "$(ls /usr/lib/llvm-*/bin/$t | head -n1)" /usr/bin/$t; \
     done

RUN rustup target add x86_64-pc-windows-msvc \
  && cargo install --locked cargo-xwin --version ${CARGO_XWIN_VERSION} \
  && rm -rf /usr/local/cargo/registry /usr/local/cargo/git

RUN curl -fsSL https://bun.sh/install | bash -s "bun-v${BUN_VERSION}" \
  && ln -s /root/.bun/bin/bun /usr/local/bin/bun \
  && ln -s /root/.bun/bin/bun /usr/local/bin/bunx \
  && ln -s /root/.bun/bin/bun /usr/local/bin/node
