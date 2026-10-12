# P9IME

A Windows 10/11 Text Services Framework (TSF) input method written in Rust,
cross-compiled from Linux.

## Install / uninstall

Download `p9ime-installer-<version>-windows-x64.exe` from the
[latest release](https://github.com/p9n/p9ime/releases/latest) and run it as
Administrator. There is nothing to unzip: the IME DLL is embedded in the
installer.

The installer

- writes `p9ime.dll` and a copy of itself (`uninstaller.exe`) to
  `C:\Program Files\P9IME\`,
- registers the COM server and the TSF profile under
  **Chinese (Traditional, Taiwan)**, and
- registers an entry in *Settings → Apps → Installed apps*.

After installing, sign out and back in, then add the keyboard via
*Settings → Time & Language → Language → 中文(台灣) → Options → Add a keyboard →
P9IME*.

To uninstall, use *Installed apps*, run
`C:\Program Files\P9IME\uninstaller.exe`, or run the installer with
`--uninstall`. Both modes accept `--quiet` / `/S` for silent operation.

Reinstalling over an existing install, and uninstalling then reinstalling
without a reboot, are both supported; files that are in use are moved aside
and cleaned up at the next reboot.

## Building

Requires a stable Rust toolchain with the `x86_64-pc-windows-gnu` target and
`mingw-w64` for linking:

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64          # Debian/Ubuntu
cargo build --release
```

Outputs in `target/x86_64-pc-windows-gnu/release/`:

| File                  | Purpose                                            |
| --------------------- | -------------------------------------------------- |
| `p9ime.dll`           | The TSF text service (in-process COM server)       |
| `p9ime-installer.exe` | Self-contained installer; embeds `p9ime.dll`       |

### Linting

The installer `include_bytes!`s the DLL, which `cargo check` / `cargo clippy`
never produce. Lint with the `embed-dll` feature disabled:

```sh
cargo lint   # alias for: clippy --all-targets --release --no-default-features -- -D warnings
```

(Running clippy with default features also works after a `cargo build`.)

## Layout

| Path                        | Contents                                              |
| --------------------------- | ----------------------------------------------------- |
| `src/lib.rs`                | COM DLL entry points (`DllGetClassObject`, …)         |
| `src/ime.rs`                | `TextService`: TSF sinks, key handling, composition   |
| `src/registration.rs`       | COM / TSF / Installed-apps registry registration      |
| `src/files.rs`              | In-use-safe file replacement and reboot cleanup       |
| `src/common.rs`             | GUIDs, names, language ID                             |
| `src/bin/installer/`        | The setup binary (install + uninstall modes)          |
| `build.rs`                  | Exposes the DLL path to the installer's `include_bytes!` |
| `.github/workflows/`        | CI; pushing a `v*` tag publishes a GitHub Release     |

## Status

The current key handling is a placeholder used to exercise the TSF
composition pipeline (buffer up to five letters, `Space` commits the last one,
`Backspace` edits, `Esc` cancels). Real IME logic will replace it.
