# Сборочный образ для Arch Linux: бинарь собирает tauri, пакет — makepkg (ci/arch/PKGBUILD).
FROM archlinux:base-devel

ARG BUN_VERSION=1.2.21
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN pacman -Syu --noconfirm --needed \
      curl wget file unzip jq git pkgconf \
      webkit2gtk-4.1 gtk3 librsvg openssl libayatana-appindicator xdg-utils \
  && pacman -Scc --noconfirm \
  # makepkg не запускается от root
  && useradd -m builder

COPY common.sh /tmp/common.sh
RUN sh /tmp/common.sh "${BUN_VERSION}"
