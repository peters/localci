# CI uses the hosted Ubuntu runner's Rust, with no repository version override.
# Source: actions/runner-images, images/ubuntu/Ubuntu2404-Readme.md.
FROM ghcr.io/peters/horizon-worker-base@sha256:9f233978dedd3d99214990ff8346e9539d4c127d5e827c74488e21346823b8f2

USER root

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential musl-tools python3 \
    && rm -rf /var/lib/apt/lists/*

ARG RUST_VERSION=1.99.0
ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71

# Install proxies and toolchains outside HOME; session Cargo caches stay in HOME.
RUN curl -fsSL "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" -o /tmp/rustup-init \
    && printf '%s  /tmp/rustup-init\n' "${RUSTUP_SHA256}" | sha256sum -c - \
    && chmod 0755 /tmp/rustup-init \
    && CARGO_HOME=/opt/rust/cargo RUSTUP_HOME=/opt/rust/rustup \
       /tmp/rustup-init -y --no-modify-path --profile minimal \
       --default-toolchain "${RUST_VERSION}" --component rustfmt --component clippy \
       --target x86_64-unknown-linux-musl \
    && chmod -R a+rX /opt/rust \
    && rm /tmp/rustup-init

# Latest published release, including prereleases, resolved at setup time.
ARG LOCALCI_VERSION=0.1.0
ARG LOCALCI_SHA256=215096651a2af9b1a8d64825d6453b7e090049078ac30525074e9e1f43fdc893
RUN mkdir -p /tmp/localci-install \
    && cd /tmp/localci-install \
    && curl -fsSL "https://github.com/peters/localci/releases/download/v${LOCALCI_VERSION}/localci-x86_64-unknown-linux-musl.tar.gz" -o localci-x86_64-unknown-linux-musl.tar.gz \
    && curl -fsSL "https://github.com/peters/localci/releases/download/v${LOCALCI_VERSION}/SHA256SUMS" -o SHA256SUMS \
    && grep '  localci-x86_64-unknown-linux-musl.tar.gz$' SHA256SUMS | sha256sum -c - \
    && printf '%s  localci-x86_64-unknown-linux-musl.tar.gz\n' "${LOCALCI_SHA256}" | sha256sum -c - \
    && tar -xzf localci-x86_64-unknown-linux-musl.tar.gz \
    && install -m 0755 localci /usr/local/bin/localci \
    && rm -rf /tmp/localci-install

RUN mkdir -p /etc/horizon-worker/session-env.d \
    && printf 'PATH_PREPEND=/opt/rust/cargo/bin\nPATH_PREPEND=/usr/local/bin\nRUSTUP_HOME=/opt/rust/rustup\n' \
       > /etc/horizon-worker/session-env.d/20-localci.env
