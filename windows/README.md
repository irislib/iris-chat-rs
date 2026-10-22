# Iris Chat — Windows shell

WPF (.NET 8) shell over the shared Rust core. Mirrors the Android/iOS/macOS
shells: thin native UI, Rust owns app state and protocol logic.

## What lives here

- `IrisChat/IrisChat.csproj` — WPF app project (self-contained `win-x64`)
- `IrisChat/AppManager.cs` — shell-side mirror of `AppManager.swift` /
  `AppManager.kt`. Builds the Rust `FfiApp`, restores secure credentials,
  forwards user actions, applies `AppUpdate`s.
- `IrisChat/WindowsCredentialStore.cs` — Windows Credential Manager backed
  store for the owner/device secret bundle (analog of iOS Keychain).
- `IrisChat/Views/` — XAML surface: Welcome, ChatList, Chat.
- `IrisChat/Bindings/` — generated C# from `uniffi-bindgen-cs`. Gitignored;
  rebuilt every `windows-gen-cs`.
- `IrisChat/Frameworks/` — Rust `iris_chat_core.dll` dropped here after Rust
  build, copied into the publish folder by MSBuild. Gitignored.

## Build target

`x86_64-pc-windows-msvc` only. The build can run inside an ARM Windows VM
(Apple Silicon Parallels) — Microsoft's x64 emulation runs the produced
binaries transparently for dev. Production ships the same x86_64 build to
real x86_64 Windows machines.

## Build harness

The whole build is driven from macOS via `prlctl` against a running
Parallels Windows VM, exactly like `nostr-vpn`'s release script. The mac
home directory is auto-mounted at `C:\Mac\Home\...` inside the guest;
`robocopy /MIR` syncs the repo (and the sibling `nostr-double-ratchet`
path-dep) into `~/src` on the guest before each build.

```
just windows-doctor    # probe VM toolchain
just windows-setup     # one-time: install .NET 8 SDK via winget
just windows-sync      # robocopy repo into VM
just windows-rust      # cargo build x86_64-pc-windows-msvc
just windows-gen-cs    # uniffi-bindgen-cs → C# bindings
just windows-dotnet    # dotnet publish self-contained
just windows-build     # full pipeline above
just run-windows       # build, then launch IrisChat.exe inside VM
```

Env knobs: see `./scripts/windows-build help`. By default, the helper tries
`windows-build` and then `win11-dev`; set `IRIS_WINDOWS_SSH_HOST` to force one
host or `IRIS_WINDOWS_SSH_HOSTS` for a custom fallback list.

## VM prerequisites

`windows-setup` installs the .NET 8 SDK. The rest is one-time manual setup
inside the VM (handled by the Parallels Tools install plus the user's prior
`nostr-vpn` setup):

- Rust toolchain (`rustup`) with the `x86_64-pc-windows-msvc` target
- Visual Studio C++ Build Tools with the **MSVC v143 - x64/x86 build tools**
  workload (provides the `link.exe` cargo needs)
- Windows SDK and .NET 8 SDK
- LLVM (provides `clang.exe` for some `cc-rs` builds; default search path is
  `C:\Program Files\LLVM\bin`)
- Parallels Tools' "User home directory" host sharing turned on

The build requires the Rust target to be installed already; it does not install
or update the toolchain. CI installs that prerequisite before invoking the same
build entrypoint. If C# binding generation needs `uniffi-bindgen-cs`, the entrypoint
installs the reviewed full Git commit for `v0.10.0+v0.29.4` with Cargo's locked
dependencies. `IRIS_WINDOWS_BINDGEN_CS_GIT_REF` overrides must be full commit
hashes; tags and branches are rejected. An already installed tool must have Cargo
metadata matching that repository and commit; the build invokes the checked
Cargo executable directly, ignoring other copies on `PATH`.

.NET restores use the committed `windows/IrisChat/packages.lock.json` in locked
mode. Changing a dependency version or package hash makes the restore fail.
NuGet does not put SDK download packages in that lock: their exact versions and
SHA-512 hashes are recorded separately in `windows/IrisChat/sdk-packages.json`.
After restoring, the entrypoint checks every SDK download archive against those
pins before publishing with `--no-restore`. Unknown SDK download packages fail
closed. Dependency updates must review and update the relevant locks and pins;
the installed SDK and native compilers remain declared build prerequisites.

Once those are present, `just windows-build` produces a self-contained
`IrisChat.exe` under
`windows/IrisChat/bin/Debug/net8.0-windows10.0.17763.0/win-x64/publish/`.

## Keyboard regression

In an interactive Windows desktop, run:

```powershell
.\scripts\test-windows-composer.ps1 -AppPath "<path to built IrisChat.exe>"
```

The test creates an isolated profile, types into the focused message field one
character at a time, checks Shift+Enter, and sends with Enter. It never uses the
clipboard or resets an existing profile.

Run the native focus and chat-list navigation checks on Windows with:

```powershell
dotnet run --project windows/KeyboardTests/KeyboardTests.csproj -c Release -- work/keyboard
```

This uses a separate test profile and checks Tab/Shift-Tab, arrow navigation,
Enter/Space activation, focus during updates, and composer isolation. It saves
`work/keyboard/windows-keyboard.png` for visual review.

## Theme regression

The app follows Windows Settings → Personalization → Colors → Choose your mode,
including changes while it is open. Run the native theme checks on Windows with:

```powershell
dotnet run --project windows/ThemeTests/ThemeTests.csproj -c Release -- work/themes
```

This uses an isolated offline profile, materializes styles before selecting the
light palette, and checks all 13 settings pages, navigation, onboarding, chat and
attachment controls in both palettes. It also checks existing controls after
live theme changes. Settings checks cover arrow/Tab navigation, returning to the
chat list, saving profile edits, notification switches, and all categories at
1100×720, 720×540, and 150% zoom. Enabled text must have at least 4.5:1 contrast; interface
icons must have at least 3:1. Avatar initials and badges are included; uploaded
image pixels are not evaluated. Screenshots
for visual review are saved to `work/themes/` and uploaded by Windows CI.
