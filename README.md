# ryoiki

Native Linux visual novel setup manager. The current source build provides:

- A GTK 4/libadwaita library home. + starts a local install or adds a VNDB title.
- The install wizard accepts archives, disc images, folders, and Windows installers. It extracts with `7z`, creates an isolated per-game Wine prefix, keeps Windows user folders inside that prefix, maps extracted disc files as `d:`, and runs the installer interactively.
- After setup, the wizard discovers new executables, asks which one to launch, and saves the launch profile in SQLite. Saved profiles load back into the library.
- Multiple discs and setup programs require an explicit choice. Physical-disc checks, copy protection workarounds, and library launching are not implemented.

## Build from source

Requirements:

- Rust 1.93 or newer
- GTK 4 and libadwaita development files
- `pkg-config`
- A C compiler and linker
- `7z`
- `wine` and `wineboot`
- `locale` and `localedef` (used to generate a private `ja_JP.UTF-8` locale when needed)

Build and run:

```sh
cargo run
```

Run the focused tests:

```sh
cargo test
```

On NixOS, the current source tree can be run without a repository flake:

```sh
nix-shell -p cargo rustc pkg-config gtk4 libadwaita gcc --run 'cargo run'
```
