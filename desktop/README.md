# GWP Annotator — Stage 2 Core

This directory contains the Stage 2 desktop foundation: a Tauri 2 shell, a Rust core, and a local SQLite data layer. The clinician-facing browser and drawing workflow are intentionally deferred to later stages.

## Scope delivered here

- Tauri 2 + native TypeScript placeholder shell with a strict offline CSP (`connect-src 'none'`).
- Read-only two-campus scan of direct case directories and direct JPG/JPEG/MP4 files.
- Date-descending case ordering, stable opaque-ID tie-breaking, and natural media ordering.
- Process-only source mapping: source names and paths are kept only in Rust memory. A dedicated local-UI IPC response exposes temporary campus `display_name` and case `display_label`; neither enters SQLite, recovery, cache, export, or logs.
- HMAC-derived `CASE-...` and `ASSET-...` identifiers; tests inject a fixed secret. The production provider uses the operating-system credential store, isolated from the project work directory.
- ProjectService rejects identical, nested, and canonicalized symlink/junction-overlapping source/work roots before scanning or writing. The SQLite work directory is opened lazily, after that check, so an unsafe configuration cannot create a database inside a source tree.
- SQLite migrations, WAL, `synchronous=FULL`, representative-asset transaction limit (two per case), template snapshots, and one confirmed-region transaction. A representative with saved regions cannot be deselected.
- Derived-file writes validate every output-directory component, reject symlinks and Windows reparse points, verify the resolved parent remains under the work root, and use a same-directory temporary file. Existing Windows targets are replaced through `ReplaceFileW`; non-Windows uses same-volume rename semantics.
- Database startup reserves and validates `annotations.sqlite3`, `annotations.sqlite3-journal`, `annotations.sqlite3-wal`, and `annotations.sqlite3-shm` as regular work-root files before SQLite opens them, re-reserves and validates the full set after migration/WAL setup, and does so before every core database operation. The primary database uses SQLite's no-follow open flag. SQLite exposes no separate rusqlite no-follow handle for every later rollback-journal/WAL/SHM VFS open, so a hostile same-user rename exactly between those checks and a SQLite sidecar open remains outside the atomic guarantee; the work root must remain application-controlled.
- JSON label-template validation plus simplified `label.txt` import. No default domain vocabulary or template is bundled.

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
- The project work directory is selected through Tauri's local app-data path. The selected source root is never a write target.
- Windows overwrite behavior is implemented with `ReplaceFileW` for existing derived files. It is covered by platform-neutral repeated-write tests here, but it must still be exercised on a Windows reference device; macOS verification is not Windows runtime evidence.
- `rust:check:windows` is a best-effort compile check of the `x86_64-pc-windows-msvc` target using the project-local Rust toolchain. It is not Windows runtime evidence.
- IPC commands use product semantics such as `scan_selected_root` and `deselect_representative`.
- The current Stage 2 core scans MP4 metadata for browsing order but does not implement video playback, frame extraction, export, or any clinical UI.

## Test fixture policy

All fixtures are created at test runtime under temporary folders. Their directory and file names use only `ROOT`, `CAMPUS`, `CASE`, and `ASSET` placeholders. No user-provided source-media directory is read by the test suite.
