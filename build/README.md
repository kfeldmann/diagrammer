# build/

Docker-based release builds. Each script builds its image, then runs it with
the project directory mounted at `/work`, so artifacts land in the host's
`target/` tree and are owned by you.

```bash
build/build-linux-glibc   # x86_64-unknown-linux-gnu, dynamically linked
build/build-linux-musl    # x86_64 musl, dynamically linked
build/build-mac           # aarch64-apple-darwin (Apple Silicon)
build/build-windows       # x86_64-pc-windows-gnu (diagrammer.exe)
```

| Script               | Image              | Command                                              | Artifact                                              |
| -------------------- | ------------------ | ---------------------------------------------------- | ----------------------------------------------------- |
| `build-linux-glibc`  | `debian.Dockerfile`| `cargo zigbuild --target x86_64-unknown-linux-gnu.2.26` | `target/x86_64-unknown-linux-gnu/release/diagrammer`  |
| `build-linux-musl`   | `alpine.Dockerfile`| `cargo build` (native musl)                          | `target/release/diagrammer`                           |
| `build-mac`          | `debian.Dockerfile`| `cargo zigbuild --target aarch64-apple-darwin`       | `target/aarch64-apple-darwin/release/diagrammer`      |
| `build-windows`      | `debian.Dockerfile`| `cargo zigbuild --target x86_64-pc-windows-gnu`      | `target/x86_64-pc-windows-gnu/release/diagrammer.exe` |

Only the `diagrammer` package is built (`-p diagrammer`); the
`gen-metrics-table` workspace member is not a release artifact.

## Design notes

- **Shared Debian image.** `linux-glibc`, `mac`, and `windows` use the same
  image; the scripts distinguish them by passing a different cargo command to
  `docker run`. `linux-musl` is separate because Alpine is musl-based.
- **Runs as the host user.** Each script passes `--user "$(id -u):$(id -g)"`
  so files written into the mounted project keep your ownership. Because an
  arbitrary UID can't write to the image's toolchain directories, `CARGO_HOME`
  and `HOME` are redirected into the gitignored `target/` directory
  (`target/.cargo-home`, `target/.home`); this also persists the crate cache
  between builds.
- **"Shared" = dynamically linked.** For glibc that is `cargo zigbuild` with
  an explicit `x86_64-unknown-linux-gnu.2.26` triple, which caps the minimum
  glibc version at 2.26 regardless of the Debian version inside the image.
- **Toolchain choice.** The Debian image uses rustup rather than Debian's
  `rustc` because the macOS cross build needs the `aarch64-apple-darwin` std,
  which only rustup can add. The Alpine image uses the distro `rust`/`cargo`
  packages, since the build is native.
- **No macOS SDK.** `cargo-zigbuild` is installed from PyPI, which also pulls
  the bundled `ziglang` wheel. Zig ships the libSystem stubs needed to link
  this pure-Rust binary.
- **Windows is `-gnu`, not `-msvc`.** Both targets produce a working `.exe`,
  but zig cannot drive the MSVC linker (`--target x86_64-pc-windows-msvc`
  fails with `/NOLOGO: unrecognized file extension`), and the gnu target has
  the useful side effect of statically linking the MinGW runtime: the `.exe`
  needs no `libgcc_s_seh-1.dll` or `libwinpthread-1.dll` beside it. Its only
  imports are OS components (`KERNEL32`, `ntdll`, `WS2_32`, `USERENV`,
  `bcryptprimitives`, `api-ms-win-crt-*`), so it is a single-file download.
  It is a console-subsystem PE32+ binary, so stdout/stderr and exit codes work
  in `cmd` and PowerShell.

## Distributing the Windows binary

The `.exe` is **unsigned**, and that is fine for most uses. What it does and
doesn't cost:

- **Runs normally.** Windows does not require a signature to execute a binary.
  Unzipped, copied over `scp`/`git clone`, or built locally, `diagrammer.exe`
  just runs.
- **Downloaded → SmartScreen.** A file that arrives with Mark-of-the-Web (a
  browser or mail download; zipping doesn't help, the mark propagates to
  extracted files) may show "Windows protected your PC". That's one extra
  click on *More info → Run anyway* — a warning, not a block.
- **Smart App Control** is the one real block, and it has no per-app override.
  It is on only for clean Windows 11 installs. Since the April 2026 update it
  is reversible, unlike the old one-way door.
- **Enterprise WDAC / App Control** whitelists only signed binaries; nothing
  here helps if a user is subject to one.

If signing is ever wanted, the options (as of late 2026):

| Option | Cost | Notes |
| --- | --- | --- |
| **SignPath Foundation** | free | For open-source projects; they verify the binary was built from the repo and sign via their HSM. Best fit for this MIT-licensed tool. |
| Azure Artifact Signing (was Trusted Signing) | ~$9.99/mo | Individuals must be in the US or Canada; needs a *paid* Azure subscription (free/trial/sponsored rejected). |
| OV certificate | $150–300/yr | Hardware token required. |
| EV certificate | $400+/yr | No longer bypasses SmartScreen (changed in 2024) — it builds reputation like OV. |
| Self-signed | free | Only useful with managed enterprise trust. |

Note that **any** new signature starts at zero SmartScreen reputation, so
signing reduces friction but does not remove the first-run warning.

## Adding a platform

1. Add a `build-<name>` script (copy the closest one, change the image/command
   and the echoed artifact path).
2. If it uses a new base image, add a `*.Dockerfile`; if it shares Debian or
   Alpine, reuse the existing one and just pass a different command.
