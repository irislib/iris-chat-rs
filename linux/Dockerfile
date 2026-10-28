# Dev container for the Linux GTK4/libadwaita shell.
# Ubuntu 26.04 ships a recent packaged Rust toolchain; rustup's x86_64 rustc
# currently crashes under Docker/QEMU on Apple Silicon release hosts.
# Runs Xvfb + fluxbox + x11vnc so the app window is reachable from the host on
# vnc://localhost:5900 (no password).

FROM ubuntu:26.04@sha256:c098726dd9b9733f3e885f8de0ef375206a311b0a317eadc40d347b1115fb944

# The minimal image needs CA trust before HTTPS APT bootstrap. BuildKit verifies
# this dated Mozilla bundle before use; Ubuntu ca-certificates supplies normal
# trust below. Use existing /tmp so ADD's file mode cannot restrict parent dirs.
ADD --checksum=sha256:796f1389bfea13c0ed78f663b765baaff54a508e5483786bd6ece73c5e3e3cbe --chmod=0644 \
    https://curl.se/ca/cacert-2026-10-20.pem /tmp/iris-bootstrap-ca.pem

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    DISPLAY=:99 \
    GDK_BACKEND=x11 \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

COPY scripts/pinned-apt.sh /opt/iris-build/scripts/pinned-apt.sh
COPY build-inputs/apt-snapshots.tsv /opt/iris-build/build-inputs/apt-snapshots.tsv
RUN SSL_CERT_FILE=/tmp/iris-bootstrap-ca.pem bash /opt/iris-build/scripts/pinned-apt.sh ubuntu \
        build-essential \
        cmake \
        pkg-config \
        ca-certificates \
        curl \
        git \
        python3 \
        rustc \
        cargo \
        libgtk-4-dev \
        libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
        gstreamer1.0-plugins-base \
        gstreamer1.0-plugins-good \
        gstreamer1.0-libav gstreamer1.0-pipewire \
        libadwaita-1-dev \
        libdbus-1-dev \
        libssl-dev \
        libsecret-1-dev \
        libglib2.0-dev \
        libsoup-3.0-dev \
        libclang-dev \
        libv4l-dev \
        libasound2-dev \
        nasm \
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
        adwaita-icon-theme

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
