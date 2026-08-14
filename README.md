# ryoiki

Native Linux visual novel setup manager. The current source build provides:

- A GTK 4/libadwaita library home. Add game → archive or folder.
- File-name autodetect, then VNDB search with cover thumbnails if the name is ambiguous

The setup pipeline is still in progress. It does not yet extract media, create Wine prefixes, run installers, or save launch profiles.

## Build from source

Requirements:

- Rust 1.93 or newer
- GTK 4 and libadwaita development files
- `pkg-config`
- A C compiler and linker

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
