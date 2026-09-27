# build/

Docker-based release builds. Each script builds its image, then runs it with
the project directory mounted at `/work`, so artifacts land in the host's
`target/` tree and are owned by you.

```bash
build/build-linux-glibc   # x86_64-unknown-linux-gnu, dynamically linked
build/build-linux-musl    # x86_64 musl, dynamically linked
build/build-mac           # aarch64-apple-darwin (Apple Silicon)
```

| Script               | Image              | Command                                              | Artifact                                              |
| -------------------- | ------------------ | ---------------------------------------------------- | ----------------------------------------------------- |
| `build-linux-glibc`  | `debian.Dockerfile`| `cargo build --target x86_64-unknown-linux-gnu`      | `target/x86_64-unknown-linux-gnu/release/diagrammer`  |
| `build-linux-musl`   | `alpine.Dockerfile`| `cargo build` (native musl)                          | `target/release/diagrammer`                           |
| `build-mac`          | `debian.Dockerfile`| `cargo zigbuild --target aarch64-apple-darwin`       | `target/aarch64-apple-darwin/release/diagrammer`      |

Only the `diagrammer` package is built (`-p diagrammer`); the
`gen-metrics-table` workspace member is not a release artifact.

## Design notes

- **Shared Debian image.** `linux-glibc` and `mac` use the same image; the
  scripts distinguish them by passing a different cargo command to
  `docker run`. `linux-musl` is separate because Alpine is musl-based.
- **Runs as the host user.** Each script passes `--user "$(id -u):$(id -g)"`
  so files written into the mounted project keep your ownership. Because an
  arbitrary UID can't write to the image's toolchain directories, `CARGO_HOME`
  and `HOME` are redirected into the gitignored `target/` directory
  (`target/.cargo-home`, `target/.home`); this also persists the crate cache
  between builds.
- **"Shared" = dynamically linked.** For glibc that is rustc's default. For
  musl, rustc statically links by default, so `alpine.Dockerfile` sets
  `RUSTFLAGS="-C target-feature=-crt-static"`. Remove that `ENV` to produce the
  usual fully static musl binary.
- **Toolchain choice.** The Debian image uses rustup rather than Debian's
  `rustc` because the macOS cross build needs the `aarch64-apple-darwin` std,
  which only rustup can add. The Alpine image uses the distro `rust`/`cargo`
  packages, since the build is native.
- **No macOS SDK.** `cargo-zigbuild` is installed from PyPI, which also pulls
  the bundled `ziglang` wheel. Zig ships the libSystem stubs needed to link
  this pure-Rust binary.

## Adding a platform

1. Add a `build-<name>` script (copy the closest one, change the image/command
   and the echoed artifact path).
2. If it uses a new base image, add a `*.Dockerfile`; if it shares Debian or
   Alpine, reuse the existing one and just pass a different command.
