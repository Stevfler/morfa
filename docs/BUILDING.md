# Building Orbit

Goal: every bundle works out of the box, with no extra packages installed by the user.

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
| AppImage / .deb / .rpm | WebKitGTK 4.1 (pulled in by apt/dnf for deb/rpm, bundled in the AppImage). No media framework needed: Orbit doesn't play audio/video. |
| Windows (NSIS/MSI) | `webviewInstallMode: embedBootstrapper` (silent). The installer ships the WebView2 bootstrapper and installs WebView2 if it is missing. |
| macOS (.app/.dmg) | Nothing extra. WKWebView is part of the OS. |

## Host packages needed to build on Linux

- Tauri prerequisites: `webkit2gtk-4.1`, `base-devel`, `curl`, `wget`, `file`, `openssl`, `appmenu-gtk-module`, `librsvg`, `xdotool` (Arch names).

## Caveats

- Build the AppImage on the **oldest** distro you want to support (e.g. Ubuntu 22.04). AppImages link against host glibc, so one built on Arch may not start on older distros.
- Wayland: `src-tauri/src/main.rs` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` to avoid blank windows or crashes with some GPU drivers. That is a runtime workaround and is unrelated to bundling.
- Windows `embedBootstrapper` needs internet access on the first install if WebView2 is missing. Use `offlineInstaller` for fully offline installs, at a cost of about 127 MB.
- `.deb`/`.rpm` installed with `dpkg -i`/`rpm -i` instead of apt/dnf will not pull dependencies automatically.

## Bundled image/video tools (sidecars)

Orbit looks for `vips`, `vipsheader`, `magick` (and later `ffmpeg`) in this order: next to the Orbit executable, `<exe dir>/bin`, `<resource_dir>/bin`, then `PATH` (`src-tauri/src/sidecar.rs`). Nothing is bundled yet. To bundle:

- **Tauri `externalBin`** (single executables): put binaries in `src-tauri/binaries/` named with the target triple, e.g. `vips-x86_64-pc-windows-msvc.exe`, `vips-aarch64-apple-darwin`, and list `"externalBin": ["binaries/vips", "binaries/vipsheader", "binaries/ffmpeg"]` under `bundle` in `tauri.conf.json`. Tauri strips the triple and installs them next to the app executable.
- **Resources** (tools that need their DLLs/dylibs, e.g. libvips' Windows build): add `"resources": { "vendor/vips/bin/": "bin/" }` so they land in `<resource_dir>/bin`. On Windows the DLLs must sit in the same folder as `vips.exe`.
- Sources: libvips Windows builds from `github.com/libvips/build-win64-mxe` releases (use the `-static` variant to avoid DLL sprawl); on macOS build a relocatable vips with Homebrew deps or skip it (users can `brew install vips`). ffmpeg: static builds (gyan.dev / BtbN for Windows, evermeet.cx for macOS). Check licences (LGPL/GPL) before shipping.
- macOS: every bundled binary must be signed with the app's identity (Tauri signs `externalBin` automatically when signing is configured).

## Office module (document conversion on Windows/macOS)

Not bundled: Orbit downloads a pinned LibreOffice on demand (`src-tauri/src/office/`), unpacks it without installing (Windows: `msiexec /a` administrative image; macOS: copy out of the dmg), prunes it (`office/prune.rs`) and keeps it in `<local data dir>/Orbit/office`. To bump the version, edit `office/manifest.rs` (version + four SHA-256 values from `<url>.sha256`) and run `cargo test office -- --ignored` (HEAD-checks the URLs). To re-validate the prune list on Linux:

```sh
cp -a /usr/lib/libreoffice /tmp/lo && ORBIT_PRUNE_DIR=/tmp/lo cargo test office::tests::prune_real_install -- --ignored
/tmp/lo/program/soffice --headless --convert-to pdf some.docx   # plus xlsx/csv/pptx/html samples
```
