# Dev container for the Linux GTK4/libadwaita shell.
# Ubuntu 26.04 ships a recent packaged Rust toolchain; rustup's x86_64 rustc
# currently crashes under Docker/QEMU on Apple Silicon release hosts.
# Runs Xvfb + fluxbox + x11vnc so the app window is reachable from the host on
# vnc://localhost:5900 (no password).

FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    DISPLAY=:99 \
    GDK_BACKEND=x11 \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        ca-certificates \
        curl \
        git \
        rustc \
        cargo \
        libgtk-4-dev \
        libadwaita-1-dev \
        libssl-dev \
        libsecret-1-dev \
        libglib2.0-dev \
        libsoup-3.0-dev \
        libclang-dev \
        libv4l-dev \
        xvfb \
        x11vnc \
        fluxbox \
        dbus-x11 \
        xterm \
        x11-utils \
        xdotool \
        imagemagick \
        fonts-noto-core \
        fonts-noto-color-emoji \
        adwaita-icon-theme \
    && rm -rf /var/lib/apt/lists/*

ARG INSTALL_CARGO_WATCH=1
RUN mkdir -p "$CARGO_HOME" \
    && if [ "$INSTALL_CARGO_WATCH" = "1" ]; then cargo install --locked cargo-watch; fi \
    && chmod -R a+w "$CARGO_HOME"

WORKDIR /workspace/linux

EXPOSE 5900

COPY scripts/dev-entrypoint.sh /usr/local/bin/dev-entrypoint
RUN chmod +x /usr/local/bin/dev-entrypoint

ENTRYPOINT ["/usr/local/bin/dev-entrypoint"]
CMD ["bash"]
