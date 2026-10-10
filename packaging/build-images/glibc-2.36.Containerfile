FROM docker.io/library/rust@sha256:b36c246742b4d323472588f789601be34eb4af053255e2af53969078ee8dcec7

ARG DEBIAN_FRONTEND=noninteractive

# Debian 12 is the oldest supported build environment because it combines
# glibc 2.36 with WebKitGTK 4.1. Building every shipped executable here keeps
# accidental references to newer host libc symbols out of release artifacts.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        binutils \
        build-essential \
        libayatana-appindicator3-dev \
        libgtk-3-dev \
        librsvg2-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        nodejs \
        npm \
        pkg-config \
        python3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
