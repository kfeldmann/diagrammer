# syntax=docker/dockerfile:1
#
# Build image for the native x86_64 musl build. Kept separate from the Debian
# image because Alpine is musl-based and uses the distro toolchain directly.
#
#   build/build-linux-musl -> cargo build --release -p diagrammer
#
# The container is run with --user <host uid>:<host gid>, so nothing here
# assumes a writable toolchain install; see build/README.md.

FROM alpine:3.24

# build-base: cc/ld for linking.
# musl-dev:   musl libc headers.
# rust+cargo: the distro toolchain (native x86_64-*-musl), matching the
#             packages used by the project's own dev container.
RUN apk add --no-cache \
      build-base \
      ca-certificates \
      cargo \
      musl-dev \
      rust

# "Shared" (dynamically linked) musl binary. rustc statically links musl by
# default; disabling crt-static makes it link against the system musl
# (/lib/ld-musl-x86_64.so.1) instead. Drop this ENV for the usual fully static
# musl binary.
ENV RUSTFLAGS="-C target-feature=-crt-static"

WORKDIR /work
