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
