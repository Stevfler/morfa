# Сборочный образ для rpm (Fedora).
ARG BASE=fedora:42
FROM ${BASE}

ARG BUN_VERSION=1.4.4
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN dnf install -y \
      gcc gcc-c++ make curl wget file unzip jq git which pkgconf-pkg-config rpm-build \
      webkit2gtk4.1-devel gtk3-devel librsvg2-devel openssl-devel \
      libappindicator-gtk3-devel xdg-utils \
  && dnf clean all

COPY common.sh /tmp/common.sh
RUN sh /tmp/common.sh "${BUN_VERSION}"
