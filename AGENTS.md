# ryoiki

Linux-only visual novel **manager**. Library: VNDB, covers, launch, playtime.

What separates it: **easy setup**. Drop an archive/ISO/folder/exe, get a prefix and a launch profile, without Lutris or a terminal.

Windows is out. Do not add a second OS.

## Phases

The setup pipeline remains release-gating. The native frontend and an isolated VNDB search client may evolve alongside it; library persistence and launch flows still consume a saved profile and must wait.

### 1. Setup pipeline (now)

`identify → extract → prefix → present disc → run setup → locate exe → save profile`

Each stage is re-runnable and inspectable. The human stays at the installer window. Unattended/silent is optional later, never the contract.

First demo: one `.7z` that contains an ISO → JP Wine prefix → `D:\setup.exe` running → user picks the installed exe.

### 2. Library

Grid/list of installed VNs, launch from the saved profile, playtime, status. VNDB search and cover cache. Native, Steam, Wine, umu.

### 3. Later, if ever

Achievements (Steam via `steam://` only), route notes, stats. Not Discord, Magpie, or Locale Emulator.

## Positioning

Most launchers assume the game is already installed. They detect runners and store a prefix path. They do not create prefixes, present discs, or run installers.

ryoiki owns that path, then is also the library.

## Setup

### In

- Archive / ISO / folder / installer exe ingest
- Encoding-safe extract (CP932 filenames are normal)
- Nested ISO / multi-file disc presentation
- Prefix templates: `vn32`, `vn64` + `ja_JP.UTF-8` + CJK fonts
- Interactive `setup.exe` / `install.exe` under Wine or umu
- Installed-exe discovery under `drive_c`
- Persist a launch profile the library launches

### Out

- Storefronts, downloads, account sync
- Cracks, no-CD patches, DRM bypass, SafeDisc/SecuROM “fixes”
- “Detect engine, apply the one true winetricks list”
- Shipping library persistence or launch UI before a profile can be saved

### Pipeline

Heuristics are allowed. Guarantees are not.

1. Open `.7z` / `.rar` / `.zip` / folder (CP932)
2. Find nested `.iso` / `.mds` / disc folder
3. Present as CD: `fuseiso` **or** extract + `dosdevices/d:` typed `cdrom`
4. `wineboot` prefix, `LANG`/`LC_ALL=ja_JP.UTF-8`, run setup
5. Hunt `drive_c` for the real game exe; keep the disc if the game checks for it

Stop and ask when:

- Multiple setup exes or discs
- Installer needs disc 2+
- Volume-label / CD-audio / sector checks (CDEmu, not phase 1)
- Layout is already a portable folder (no installer)
- DRM / copy-protection is present — leave it broken, say so

Do not automate around copy protection.

## Launch profile

Setup writes this. Library reads this. One schema.

```text
exe
prefix
arch            win32 | win64
runner          wine | umu | native | steam
locale          typically ja_JP.UTF-8
disc            mount or extracted path, Wine drive letter, cdrom type
winetricks      verbs actually applied
vndb_id         optional until phase 2
notes           what the user still has to do
```

Do not invent a second schema beside this.

## Stack

Decided. Do not re-open unless the user does.

| Layer | Choice | Why |
|---|---|---|
| UI | GTK 4 + libadwaita + Relm4 | Native Linux. Wizard now, library later. Not a WebView. |
| Core | Rust | extract, processes, `dosdevices`, prefixes |
| DB | SQLite + sqlx | profiles, then library/sessions; no SeaORM |
| Metadata | VNDB Kana | phase 2; cache locally; 200 req / 5 min |
| Runtimes | `7z`, `fuseiso`, `wine`/`wineboot`, `umu-run`, `winetricks` | subprocesses, never reimplemented |

Not Electron, Tauri, Dioxus, Iced, Slint, or egui.

Distribution: build from source until releases are mature. Flatpak and Nix packaging are later; do not add manifests, portals, sandbox permissions, or host-spawn integration during phase 1.

## Architecture

```
src/           Relm4 app + Adwaita pages
src/stages/    identify, extract, prefix, disc, setup, locate, profile
```

One crate. Relm4 pages follow the current phase (wizard first, library after). No `ui/` vs `core/` split until a stage is used from CLI tests without GTK.

Rules:

- Stages own their inputs/outputs. No god-object `Installer` that hides state.
- Subprocess wrappers stay thin. Parse their output; do not shell-script in widgets.
- Prefix mutation is explicit (`wineboot`, winetricks verb list). No implicit shared prefix.
- Default to **per-game prefix**. Templates are cloned, not mutated in place.
- Disc mounts are session-scoped and always unmounted on failure/cancel.
- Library launches profiles. It does not recreate setup logic.

## Legal / safety

User-owned files only. No fetching ISOs, cracks, or patches from the network.

Do not add, recommend, or automate:

- No-CD / crack / keygen / steam-emu
- DLL overrides whose purpose is bypassing protection
- Instructions to obtain copyrighted archives

Mounting and mapping a disc the user already has is in scope.

## Agent rules

- Correctness first. Prefer boring.
- Change existing files; do not spawn parallel abstractions.
- No extra scope: no i18n, telemetry, plugin system, cover cache, list management, or library launch UI during phase 1. The isolated VNDB search surface is allowed.
- Do not add Electron, Tauri, Dioxus, Iced, Slint, egui, Svelte, or React.
- Do not vendor Wine, 7-Zip, or winetricks.
- Comments only for non-obvious why (encoding, Wine drive types, locale).
- Build the smallest end-to-end stage, then the next.

## Current implementation

- Native Relm4/libadwaita library home. Add game → archive or folder
- Source classification for archives, disc images, folders, and Windows executables
- File-name autodetect, then VNDB Kana search with in-memory cover thumbnails
- In-memory library list only; no persistence or launch yet

Next setup work is `extract → prefix → present disc`. Library launch still waits on a saved profile.

