# Сборочный образ для deb-семейства (Ubuntu / Debian). BASE задаётся из CI.
ARG BASE=ubuntu:22.04
FROM ${BASE}

ARG BUN_VERSION=1.2.21
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential curl wget file unzip jq git ca-certificates pkg-config \
      libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev \
      libayatana-appindicator3-dev patchelf xdg-utils \
  && rm -rf /var/lib/apt/lists/*

COPY common.sh /tmp/common.sh
RUN sh /tmp/common.sh "${BUN_VERSION}"
