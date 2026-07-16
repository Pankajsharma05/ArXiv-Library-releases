# Building ArXivLibrary

This is the **complete source** for ArXivLibrary. The same source builds on
macOS, Linux, and Windows — there is no separate per-platform download.

## Important: you must build *on* the target OS

Tauri compiles a native application that links against each operating system's
own webview and system libraries. Because of that:

- **macOS builds must be done on a Mac**
- **Linux builds must be done on Linux**
- **Windows builds must be done on Windows**

You cannot produce a Windows `.exe` from a Mac, or a Mac `.app` from Linux, in
any straightforward way. (True cross-compilation exists but is fragile and not
recommended.) The normal approach is to run the build on each OS you want to
ship for — a virtual machine, a spare PC, or a CI service like GitHub Actions
all work. If you want, GitHub Actions can build all three automatically from
this same source on every push; ask and a workflow file can be added.

## One-time prerequisites (all platforms)

1. **Rust** — https://rustup.rs
2. **Tauri CLI** — `cargo install tauri-cli --version "^2"`
   (this gives you the `cargo tauri` command used below)

Then each OS needs its own system dependencies — see the header comments in the
matching build script for the exact package list.

## Building

| Platform | Script | Produces |
|----------|--------|----------|
| macOS    | `./build-mac-clean.sh` | `.app`, `.dmg` |
| Linux    | `./build-linux.sh`     | `.deb`, `.AppImage`, `.rpm` |
| Windows  | **`BUILD-WINDOWS.bat`** | `.msi`, `.exe` (NSIS) |

### Windows: one file, nothing else

On Windows you only need **`BUILD-WINDOWS.bat`**. Double-click it, or run it
from a terminal:

```
BUILD-WINDOWS.bat
```

It checks for Rust and the Tauri CLI, **installs them automatically if missing**,
verifies the MSVC C++ build tools, then builds. No Node.js is required (the
frontend is plain HTML/CSS/JS with no bundler step).

The one thing it cannot auto-install is the **Microsoft C++ Build Tools** —
Microsoft's installer requires a GUI. If they're missing, the script tells you,
offers to open the download page, and exits; install the
"Desktop development with C++" workload, then re-run the script.

Each script just runs `cargo tauri build` with helpful notes; you can also run
`cargo tauri build` directly from the project root on any platform.

Output bundles land in `src-tauri/target/release/bundle/<type>/`.

## Data location

User data (the SQLite library + downloaded PDFs) is stored per-user, separate
from the app, so reinstalling or updating the app never touches your library:

- **macOS**:   `~/Library/Application Support/ArxivLibrary/`
- **Linux**:   `~/.local/share/ArxivLibrary/`  (or `$XDG_DATA_HOME/ArxivLibrary/`)
- **Windows**: `%APPDATA%\ArxivLibrary\`

## Notes

- The first build on a machine is slow (Rust compiles all dependencies from
  scratch). Later builds are much faster thanks to caching.
- Windows and Linux builds are **unsigned** by default. Users may see a
  "unknown publisher" / Gatekeeper-style warning on first launch. Code signing
  is a separate, OS-specific step (and on Windows requires a paid certificate).
- The two GitHub placeholder URLs (the in-app "Contact on GitHub" button and the
  Crossref User-Agent header) still read `https://github.com`. Replace them with
  your real URL before public distribution.
