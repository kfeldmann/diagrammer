# syntax=docker/dockerfile:1
#
# Shared build image for the native x86_64 glibc build and the macOS and
# Windows cross builds. Debian trixie (stable).
#
# The build scripts select the target by passing the cargo command to
# `docker run`:
#
#   build/build-linux-glibc -> cargo zigbuild --target x86_64-unknown-linux-gnu.2.26
#   build/build-mac         -> cargo zigbuild --target aarch64-apple-darwin
#   build/build-mac-x86_64  -> cargo zigbuild --target x86_64-apple-darwin
#   build/build-windows     -> cargo zigbuild --target x86_64-pc-windows-gnu
#
# The container is run with --user <host uid>:<host gid>, so nothing here
# assumes a writable toolchain install; see build/README.md.

FROM debian:trixie

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# build-essential: the linker (cc) used for the native glibc build.
# python3 + pip:  cargo-zigbuild is installed from PyPI, which pulls in the
#                 bundled `ziglang` wheel, so Zig needs no separate install.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      curl \
      git \
      pkg-config \
      python3 \
      python3-pip \
 && rm -rf /var/lib/apt/lists/*

# rustup rather than Debian's `rustc` package: the macOS cross builds need the
# aarch64-apple-darwin and x86_64-apple-darwin stds, and only rustup can add a
# target's std. The native x86_64-unknown-linux-gnu target is listed for
# symmetry (it is the host's default target, so adding it is a no-op).
# x86_64-pc-windows-gnu is the Windows cross target (build/build-windows) —
# zig supplies the MinGW-w64 CRT and import libraries, so no Windows SDK or
# mingw-w64 toolchain is needed.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain stable \
 && rustup target add \
      x86_64-unknown-linux-gnu \
      aarch64-apple-darwin \
      x86_64-apple-darwin \
      x86_64-pc-windows-gnu

# cargo-zigbuild + Zig (via the ziglang wheel). Zig ships the libSystem stubs,
# so no macOS SDK is needed to link this pure-Rust binary, and it ships the
# MinGW-w64 CRT for the Windows cross build.
RUN pip install --no-cache-dir --break-system-packages cargo-zigbuild

WORKDIR /work
