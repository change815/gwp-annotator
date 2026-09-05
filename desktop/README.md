# GWP Annotator Desktop

This directory contains the current Tauri 2 desktop application, Rust core, clinician workbench, and local SQLite archive for offline image/video annotation.

## Scope delivered here

- Tauri 2 + native TypeScript/Canvas clinician workbench with a strict offline CSP (`connect-src 'none'`).
- Read-only scan of one or more arbitrarily named campus directories and their direct case JPG/JPEG/MP4 files.
- Date-descending case ordering, stable opaque-ID tie-breaking, and natural media ordering.
- Process-only source mapping: source names and paths are kept only in Rust memory. A dedicated local-UI IPC response exposes temporary campus `display_name` and case `display_label`; neither enters SQLite, recovery, cache, export, or logs.
- HMAC-derived `CASE-...` and `ASSET-...` identifiers; tests inject a fixed secret. The production provider uses the operating-system credential store, isolated from the project work directory.
- ProjectService rejects identical, nested, and canonicalized symlink/junction-overlapping source/work roots before scanning or writing. The SQLite work directory is opened lazily, after that check, so an unsafe configuration cannot create a database inside a source tree.
- SQLite schema v4 migrations, WAL, `synchronous=FULL`, unlimited representative-asset selection, immutable template snapshots, audited region correction, and atomic source-pixel boundary refinement. A representative with saved regions cannot be deselected.
- Confirming a label writes immediately to the live SQLite archive. Completing a case atomically updates one project-wide `backups/current` SQLite/template/captures snapshot; normal startup and manual project reload continue to read the newer live archive.
- Derived-file writes validate every output-directory component, reject symlinks and Windows reparse points, verify the resolved parent remains under the work root, and use a same-directory temporary file. Existing Windows targets are replaced through `ReplaceFileW`; non-Windows uses same-volume rename semantics.
- Database startup reserves and validates `annotations.sqlite3`, `annotations.sqlite3-journal`, `annotations.sqlite3-wal`, and `annotations.sqlite3-shm` as regular work-root files before SQLite opens them, re-reserves and validates the full set after migration/WAL setup, and does so before every core database operation. The primary database uses SQLite's no-follow open flag. SQLite exposes no separate rusqlite no-follow handle for every later rollback-journal/WAL/SHM VFS open, so a hostile same-user rename exactly between those checks and a SQLite sidecar open remains outside the atomic guarantee; the work root must remain application-controlled.
- Strict UTF-8 TOML label-template validation plus first-import `label.txt` compatibility. The doctor-approved study template is committed as [`templates/label-template-v1.toml`](templates/label-template-v1.toml): its three required fields render as flat radio groups (`single_select`), rather than dropdowns. The generic engine retains `dropdown` and `multi_select` support only for historical or compatibility templates.

## Local build and test

The repository-local registry settings use verified mainland-China mirrors:

- npm: `https://registry.npmmirror.com/`
- Cargo sparse registry: `https://rsproxy.cn/index/`

From `desktop/` in a clean checkout on macOS or Linux:

```bash
sh scripts/with-project-npm.sh ci
sh scripts/with-project-npm.sh run rust:bootstrap
sh scripts/with-project-npm.sh run rust:fmt
sh scripts/with-project-npm.sh run rust:clippy
sh scripts/with-project-npm.sh run rust:test
sh scripts/with-project-npm.sh run rust:build
sh scripts/with-project-npm.sh run rust:check:windows
sh scripts/with-project-npm.sh run test
sh scripts/with-project-npm.sh run build
sh scripts/with-project-npm.sh run smoke
sh scripts/with-project-npm.sh run tauri:package
```

`with-project-npm.sh` sets `npm_config_cache=desktop/.npm-cache`, keeping npm cache and error logs in the project rather than `~/.npm`; use it for every npm invocation. `preinstall` runs a Node precheck matching Vite's locked compatibility range: `^20.19.0 || >=22.12.0`. `rust:bootstrap` pins Rust `1.97.1`, verifies the rsproxy Rustup and sparse-Cargo endpoints before download, installs only below `desktop/.local-rust/`, and passes `--no-modify-path`. It neither installs global software nor changes user-level configuration. `with-local-rust.sh` and `with-local-tauri.sh` use only that toolchain and the project rsproxy Cargo source configuration. `tauri:package` uses the version-locked local `@tauri-apps/cli` to produce a macOS `.app`; it does not build a Windows installer. The generated local toolchain, Cargo cache, `src-tauri/target/`, `.npm-cache/`, `node_modules/`, and `dist/` are disposable and ignored.

If a memory-constrained macOS build is terminated with `Killed: 9`, retry only that command with project-local process settings:

```bash
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 sh scripts/with-project-npm.sh run rust:build
```

`smoke` is a static built-frontend smoke test. It checks the mount point and every text file under `dist/` for `http://` or `https://`; it is not a Tauri end-to-end test.

## Windows runtime notes

- Windows 10/11 deployments need WebView2 Runtime available from the offline installation media. The application does not fetch WebView2, codecs, models, templates, or cloud resources at runtime.
- The production secret provider uses the platform credential store; the project work directory contains only derived data and the SQLite database.
- The **数据保存位置** panel shows the active directory and offers **更换保存位置** through the native folder picker. The selected directory directly contains the annotation database, templates, captures, session and backups. Switching immediately opens that directory's project; an empty directory starts a new project. Existing files remain in the previous directory, which can be selected again to resume work. The selection is stored in `storage-location.json` under Tauri's per-user app configuration directory and is reused on startup. Without a selection, Windows uses `GWP Annotator Data/PROJECT-LOCAL` beside the installation folder. Invalid or unwritable selections leave the active project unchanged. Source/work overlap and existing derived-path protections still apply; no data is automatically moved or copied.
- Windows overwrite behavior is implemented with `ReplaceFileW` for existing derived files. It is covered by platform-neutral repeated-write tests here, but it must still be exercised on a Windows reference device; macOS verification is not Windows runtime evidence.
- `rust:check:windows` is a best-effort compile check of the `x86_64-pc-windows-msvc` target using the project-local Rust toolchain. It is not Windows runtime evidence.
- IPC commands use product semantics such as `scan_selected_root` and `deselect_representative`.
- The current application plays locally supported MP4 files and persists current-frame PNG captures under the application work directory; it does not modify source media or implement video tracking.

## Test fixture policy

All fixtures are created at test runtime under temporary folders. Their directory and file names use only `ROOT`, `CAMPUS`, `CASE`, and `ASSET` placeholders. No user-provided source-media directory is read by the test suite.
