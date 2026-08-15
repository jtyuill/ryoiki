# ryoiki

Native Linux visual novel setup manager. The current source build provides:

- A GTK 4/libadwaita library home. Adding a game starts with VNDB metadata; users can add it without files or continue into installation.
- Library entries persist independently from launch profiles. Metadata can be replaced from VNDB or edited manually, and files can be selected later.
- The install wizard accepts archives, disc images, folders, and Windows installers. It extracts with `7z`, creates an isolated per-game Wine prefix, keeps Windows user folders inside that prefix, maps extracted disc files as `d:`, and runs the installer interactively.
- After setup, the wizard discovers new executables, asks which one to launch, and attaches the saved launch profile to the existing library entry. Normal cards focus on cover, title, and launch; edit mode exposes metadata, ordering, library-only removal, and explicit deletion of ryoiki-managed Wine prefixes.
- Multiple discs and setup programs require an explicit choice. Physical-disc checks and copy-protection workarounds are not implemented.

## Build from source

Requirements:

- Rust 1.93 or newer
- GTK 4 and libadwaita development files
- `pkg-config`
- A C compiler and linker
- `7z`
- `wine` and `wineboot`
- `locale` and `localedef` (used to generate a private `ja_JP.UTF-8` locale when needed)
- `Noto Sans CJK JP` (used for Japanese Wine UI and titlebar glyphs)

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
