# Windows Stage 2.5 Validation Prompt

Paste the following prompt into Codex on a Windows reference machine. This validation covers only Stage 2: the Tauri shell and the local Rust/SQLite core. The clinician-facing browser, representative-image selection, freehand lasso, label dialog, video playback, and frame capture are later-stage work and must not be treated as missing functionality in this validation.

```text
You are the Windows native validation executor for the private repository
https://github.com/change815/gwp-annotator.

Your sole task is Stage 2.5 validation: prove or precisely report whether the
current Stage 2 Tauri/Rust local-core implementation can build, package,
install, and start on this Windows computer. Do not begin Stage 3 and do not
add clinician UI, image annotation, video playback, model inference, cloud
features, accounts, or medical labels.

Non-negotiable privacy rules:
1. Never open, enumerate, copy, hash, upload, or modify real patient folders,
   clinical JPG/JPEG/PNG/DICOM images, or surgery videos.
2. Use only repository tests and a synthetic temporary tree whose names use
   ROOT, CAMPUS, CASE, and ASSET placeholders. Put it under %TEMP%, never in a
   clinical data location.
3. Never commit build outputs, installers, SQLite databases, generated media,
   credentials, tokens, or user profile paths.
4. Do not print secrets. Use the normal authenticated Git/GitHub flow only to
   clone this private repository.

Repository and branch:
- Clone the repository into a new non-clinical working folder, for example
  C:\work\gwp-annotator.
- Record the checked-out commit SHA and do not modify tracked source files
  during a pure validation run.

Prerequisite checks (report versions and missing items before building):
- Windows 10/11 x64.
- Node.js >= 22.12 (Node 22 LTS is preferred).
- Rust stable MSVC toolchain, with `rustc`, `cargo`, and `cl` available.
- Visual Studio 2022 Build Tools: Desktop development with C++, MSVC v143, and
  a Windows 10/11 SDK.
- Git for Windows, with `sh` on PATH. The present Tauri build configuration
  calls a repository shell script before building the frontend.
- Microsoft Edge WebView2 Runtime. It is needed to run the packaged app; it is
  not an application runtime download.

Do not silently install system-wide prerequisites. If one is missing, report
BLOCKED with the exact missing component and a concise installation instruction.

From PowerShell, run the following after a successful clone:

  Set-Location C:\work\gwp-annotator\desktop
  node --version
  npm --version
  rustc -Vv
  cargo -V
  where.exe cl
  where.exe sh
  git -C .. rev-parse HEAD

Use the project-local npm cache and run all checks:

  $env:npm_config_cache = "$PWD\.npm-cache"
  npm ci
  npm run check:node
  npm run test
  npm run build
  npm run smoke
  cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
  cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings
  cargo test --manifest-path src-tauri\Cargo.toml
  cargo build --manifest-path src-tauri\Cargo.toml

The repository pins npm and Cargo registry mirrors. Dependency downloads are
allowed only during setup/build. They are not evidence of application runtime
networking.

Then create a Windows NSIS installer with the system Windows Rust toolchain:

  npx --no-install tauri build --bundles nsis

Do not use `npm run tauri:package`; that script is intentionally for the
project-local macOS/Linux Rust bootstrap. If `sh` is absent, stop before trying
to change source code and report the blocker. If the build fails for a
Windows-specific wrapper issue, preserve the exact error first; make no broad
refactor or feature work.

Find the generated installer below
`src-tauri\target\release\bundle\nsis\`. Record its full path, size, and SHA-256.
Install it with the standard interactive installer when possible, or use `/S`
only after recording that the silent install is being used for this test. Locate
the installed `GWP Annotator.exe`, start it, and confirm that a visible window
opens with:

  GWP Annotator
  Local core is ready. The clinician interface is delivered in a later stage.
  Offline by default

This is expected for Stage 2. It is not expected to browse patients or draw
annotations yet.

After all build dependencies are present, repeat the installed-app start while
Windows is temporarily offline through a user-controlled network disconnect.
Do not disable network adapters yourself. Confirm the application still opens.
While it is open, collect a best-effort 30-second check for non-loopback
established TCP connections owned by the application process and its child
processes. Report the exact command/result. A successful build-time download
does not count against this runtime check.

For the synthetic-source safety evidence, rely on the native `cargo test` run
and, if a separate synthetic tree is created, snapshot its contents and mtimes
before and after; show that the source tree is unchanged and that no test uses
real patient data.

Return one final report in Chinese, with no secrets and no patient data:

1. Overall: PASS, PARTIAL, or BLOCKED.
2. Repository URL, commit SHA, Windows version, and every prerequisite version.
3. Each command with exit code and a one-line result.
4. Test totals, especially the native Windows `cargo test` result.
5. Full NSIS installer path, size, SHA-256, installation result, installed EXE
   path, and visible-start result.
6. Offline-start and network-observation evidence.
7. Confirmation that only synthetic/repository fixtures were used and that no
   real patient media was accessed.
8. If not PASS: the first blocking error, a short diagnosis, and the smallest
   next action. Do not guess or claim Windows success from a macOS build.
```
