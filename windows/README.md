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
- `IrisChat/Frameworks/` — Rust `ndr_demo_core.dll` dropped here after Rust
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

Env knobs: see `./scripts/windows-build help`.

## VM prerequisites

`windows-setup` installs the .NET 8 SDK. The rest is one-time manual setup
inside the VM (handled by the Parallels Tools install plus the user's prior
`nostr-vpn` setup):

- Rust toolchain (`rustup`) with the `x86_64-pc-windows-msvc` target
- Visual Studio C++ Build Tools with the **MSVC v143 - x64/x86 build tools**
  workload (provides the `link.exe` cargo needs)
- Windows SDK
- LLVM (provides `clang.exe` for some `cc-rs` builds; default search path is
  `C:\Program Files\LLVM\bin`)
- Parallels Tools' "User home directory" host sharing turned on

Once those are present, `just windows-build` produces a self-contained
`IrisChat.exe` under
`windows/IrisChat/bin/Debug/net8.0-windows/win-x64/publish/`.
