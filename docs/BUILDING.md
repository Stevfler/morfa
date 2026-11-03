# Building Morfa

Goal: every bundle works out of the box, with no extra packages installed by the user.

## Version

The single source of the app version is `package.json` (`tauri.conf.json` points at it). Release CI overwrites it from the tag (`v1.2.3` → `1.2.3`), so tagging is all a release needs.

## Commands

```sh
bun install
bun run tauri build                    # all targets for the host OS
bun run tauri build --bundles appimage # or: deb, rpm (Linux) / nsis, msi (Windows) / app, dmg (macOS)
```

Output: `src-tauri/target/release/bundle/<format>/`.

## CI

`.github/workflows/ci.yml` runs on every push to `main`, every PR, and manually. Matrix: `ubuntu-22.04`, `macos-latest`, `windows-latest`. Each job runs `bun run build` (type-check + vite), `cargo clippy --all-targets -- -D warnings`, `cargo test` and `bun run tauri build`, then uploads the bundles as artifacts.

## Quick cross-OS check on Linux (no CI needed)

Compiles and lints the Windows and macOS code paths locally. It doesn't link or run them, so it catches compile errors, not runtime bugs.

```sh
rustup target add x86_64-pc-windows-msvc aarch64-apple-darwin   # once
cd src-tauri
cargo clippy --all-targets --target x86_64-pc-windows-msvc -- -D warnings
CC_aarch64_apple_darwin=clang cargo clippy --all-targets --target aarch64-apple-darwin -- -D warnings
```

`CC_…=clang` is needed because `objc2-exception-helper` compiles a C file with Apple-only flags that GCC rejects.

## What each OS needs at runtime

| Target | Runtime handling |
|---|---|
| AppImage / .deb / .rpm | WebKitGTK 4.1 (pulled in by apt/dnf for deb/rpm, bundled in the AppImage). No media framework needed: Morfa doesn't play audio/video. |
| Windows (NSIS/MSI) | `webviewInstallMode: embedBootstrapper` (silent). The installer ships the WebView2 bootstrapper and installs WebView2 if it is missing. |
| macOS (.app/.dmg) | WKWebView is part of the OS. vips and ffmpeg are bundled (see below). |

## Host packages needed to build on Linux

- Tauri prerequisites: `webkit2gtk-4.1`, `base-devel`, `curl`, `wget`, `file`, `openssl`, `appmenu-gtk-module`, `librsvg`, `xdotool` (Arch names).

## Caveats

- Build the AppImage on the **oldest** distro you want to support (e.g. Ubuntu 22.04). AppImages link against host glibc, so one built on Arch may not start on older distros.
- Wayland: `src-tauri/src/main.rs` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` to avoid blank windows or crashes with some GPU drivers. That is a runtime workaround and is unrelated to bundling.
- Windows `embedBootstrapper` needs internet access on the first install if WebView2 is missing. Use `offlineInstaller` for fully offline installs, at a cost of about 127 MB.
- `.deb`/`.rpm` installed with `dpkg -i`/`rpm -i` instead of apt/dnf will not pull dependencies automatically.

## Bundled image/video tools (sidecars)

Morfa looks for `vips`, `vipsheader`, `magick`, `ffmpeg` in this order: next to the Morfa executable, `<exe dir>/bin`, `<resource_dir>/bin`, `<resource_dir>/vips/bin`, then `PATH` (`src-tauri/src/sidecar.rs`).

Bundled today:

- **Windows** (`tauri.windows.conf.json`): `scripts/fetch-vips-win.ts`, `scripts/fetch-ffmpeg-win.ts` → `<resource_dir>/bin`.
- **macOS** (`tauri.macos.conf.json`, merged automatically on a Mac): `scripts/bundle-vips-mac.ts` copies Homebrew's vips (`brew install vips` is required on the build machine) with all non-system dylibs and the heif/jxl/poppler modules into `src-tauri/vendor/vips-mac`, rewrites load paths to `@loader_path` and ad-hoc signs everything → `<resource_dir>/vips` (Morfa sets `VIPSHOME` so modules are found). The bundle's architecture is Homebrew's. `scripts/fetch-ffmpeg-mac.ts` downloads a static ffmpeg (ffmpeg.martin-riedl.de) for `TAURI_ENV_ARCH` → `<resource_dir>/bin`. Both run from `beforeDevCommand`/`beforeBuildCommand`; run them once by hand before a bare `cargo clippy`/`cargo test`, because tauri-build checks that resources exist.

Notes on adding more:

- **Tauri `externalBin`** (single executables): put binaries in `src-tauri/binaries/` named with the target triple, e.g. `vips-x86_64-pc-windows-msvc.exe`, `vips-aarch64-apple-darwin`, and list `"externalBin": ["binaries/vips", "binaries/vipsheader", "binaries/ffmpeg"]` under `bundle` in `tauri.conf.json`. Tauri strips the triple and installs them next to the app executable.
- **Resources** (tools that need their DLLs/dylibs, e.g. libvips' Windows build): add `"resources": { "vendor/vips/bin/": "bin/" }` so they land in `<resource_dir>/bin`. On Windows the DLLs must sit in the same folder as `vips.exe`.
- Sources: libvips Windows builds from `github.com/libvips/build-win64-mxe` releases (use the `-static` variant to avoid DLL sprawl); on macOS build a relocatable vips with Homebrew deps or skip it (users can `brew install vips`). ffmpeg: static builds (gyan.dev / BtbN for Windows, evermeet.cx for macOS). Check licences (LGPL/GPL) before shipping.
- macOS: every bundled binary must be signed with the app's identity (Tauri signs `externalBin` automatically when signing is configured).

## Office module (document conversion on Windows/macOS)

Not bundled: Morfa downloads a pinned LibreOffice on demand (`src-tauri/src/office/`), unpacks it without installing (Windows: `msiexec /a` administrative image; macOS: copy out of the dmg), prunes it (`office/prune.rs`) and keeps it in `<local data dir>/Morfa/office`. To bump the version, edit `office/manifest.rs` (version + four SHA-256 values from `<url>.sha256`) and run `cargo test office -- --ignored` (HEAD-checks the URLs). To re-validate the prune list on Linux:

```sh
cp -a /usr/lib/libreoffice /tmp/lo && MORFA_PRUNE_DIR=/tmp/lo cargo test office::tests::prune_real_install -- --ignored
/tmp/lo/program/soffice --headless --convert-to pdf some.docx   # plus xlsx/csv/pptx/html samples
```
