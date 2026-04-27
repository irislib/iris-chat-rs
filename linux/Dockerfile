# Dev container for the Linux GTK4/libadwaita shell.
# Ubuntu 24.04 ships GTK 4.14 and libadwaita 1.5, which match the crate features.
# Runs Xvfb + fluxbox + x11vnc so the app window is reachable from the host on
# vnc://localhost:5900 (no password).

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    DISPLAY=:99 \
    GDK_BACKEND=x11 \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/rustup/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        ca-certificates \
        curl \
        git \
        libgtk-4-dev \
        libadwaita-1-dev \
        libssl-dev \
        libsecret-1-dev \
        libglib2.0-dev \
        libsoup-3.0-dev \
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

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal \
    && cargo install --locked cargo-watch \
    && chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"

WORKDIR /workspace/linux

EXPOSE 5900

COPY linux/scripts/dev-entrypoint.sh /usr/local/bin/dev-entrypoint
RUN chmod +x /usr/local/bin/dev-entrypoint

ENTRYPOINT ["/usr/local/bin/dev-entrypoint"]
CMD ["bash"]
