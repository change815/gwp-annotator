# Windows Stage 3 标注工作台复测指令

将下面的完整提示词交给 Windows Codex 操作员执行。它只验证当前 `main` 的 Stage 3 本地工作台，不要求也不接受真实患者资料。

```text
你是 change815/gwp-annotator 私有仓库的 Windows Stage 3 复测执行员。

你的唯一任务是：在 Windows 上核对当前 main 分支的提交、完成前端和 Rust
检查，构建 NSIS 安装包，安装并启动当前 Stage 3 标注工作台，然后用仓库测试
和合成数据给出可复核的 PASS、PARTIAL 或 BLOCKED 报告。你是在验证当前实现，
不是开发新功能；不要改动受跟踪的源文件，不要提交或推送。

## 复测范围和当前实现边界

本次范围严格是当前 Stage 3 annotation workbench：

1. 通过“选择数据根目录”打开本地目录。当前扫描要求根目录直接包含两个院区目录，
   院区下直接包含病例目录，病例目录中的 JPG/JPEG 和 MP4 媒体按序列显示；
   不要把递归扫描、导出目录或其他格式当作本次能力。
2. 在院区下浏览病例，在当前病例的媒体序列中用“上一张”“下一张”按钮或左右
   方向键浏览图片；输入框、下拉框和标签对话框获得焦点时，方向键不应抢走输入。
3. 图片可用“设为代表”选择为代表图，每个病例最多两张。尝试选择第三张时，
   预期保留前两张并报告“每个病例最多选择两张代表图。”这一当前错误提示。
4. 在已经选为代表图且已加载标签模板的图片上，按住鼠标左键自由圈选，回到起点
   附近时自动闭合。闭合后出现由外部 JSON 或 label.txt 模板生成的标签对话框。
   必填项未完成时 OK 应不可用；填写有效选项后用 OK 提交。
5. 标签对话框必须实际验证“填写区域标签”、动态模板控件、OK、取消按钮和 Esc：
   OK 应保存区域与标签；取消或 Esc 应丢弃当前尚未保存的圈选。之后切换到
   另一媒体/病例再切回，检查“已保存区域”列表仍显示已确认区域，且取消/Esc
   没有增加区域。
6. MP4 目前是明确的已知限制：当前界面显示 MP4 及“视频将在下一阶段处理。当前
   不可播放、不可截帧、不可标注。”这不是本阶段失败，也不能把它当作视频播放
   或截帧成功。

不要承诺或验收源码尚未交付的功能：视频播放、视频帧捕获、模型预测、训练导出、
账号/登录、云服务、在线模型、医学标签词表、审核流或其他医疗诊断能力。标签
字段和选项只能来自本次加载的合成模板；不要自行补充“白斑形状”“置信度”或任何
其他医学选项。

## 隐私和合成数据硬性规则

- 绝不打开、枚举、复制、读取、哈希、上传、修改或截图任何真实患者目录、临床
  JPG/JPEG/PNG/DICOM 图片或手术视频；不要在文件搜索中把患者媒体目录作为路径。
- 只使用仓库测试/仓库内合成 fixture，以及下方在 `%TEMP%` 建立的全新合成目录。
  合成目录名称和病例名称只能使用 `GWP-SYNTHETIC`、`CAMPUS-A`、`CAMPUS-B`、
  `CASE-ALPHA`、`CASE-BETA`、`ASSET-*` 等占位符。
- 终端、截图、日志和最终报告不得出现真实患者姓名、病例目录名、原始媒体路径或
  原始媒体哈希；报告中只写合成路径、仓库路径、安装器路径和脱敏的错误信息。
- 合成 JPG 只用下方本地 Base64 内容生成；合成 MP4 只是扩展名为 `.MP4` 的占位
  文件，仅用于观察当前延后提示，不能声称其可播放。不要把任何真实媒体复制进来。
- 不要把 `node_modules`、`.npm-cache`、`src-tauri\target`、安装器、SQLite 数据库、
  合成模板或日志提交到 Git。不要改动 Git 配置、代理、TLS/Schannel/CRL 配置。

## 获取并核对当前 main

优先使用普通的、已授权的 Git/GitHub 登录流程，不打印 token、密码或其他秘密。
已有工作目录若不是干净的 Git 仓库，立即报告 BLOCKED；不要使用 reset、clean、
强制覆盖或其他会丢失修改的命令。

在普通 PowerShell 中执行：

  $repoUrl = 'https://github.com/change815/gwp-annotator.git'
  $repoDir = 'C:\work\gwp-annotator'
  if (Test-Path -LiteralPath $repoDir) {
    if (-not (Test-Path -LiteralPath (Join-Path $repoDir '.git'))) {
      Write-Error "已有路径不是 Git 仓库：$repoDir"
      exit 2
    }
    $dirty = git -C $repoDir status --porcelain
    if ($dirty) {
      Write-Error '工作树不干净；请先由所有者处理现有修改。'
      exit 2
    }
    git -C $repoDir fetch origin
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    git -C $repoDir switch main
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    git -C $repoDir pull --ff-only origin main
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  } else {
    git clone --branch main $repoUrl $repoDir
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  }
  $checkedOutSha = git -C $repoDir rev-parse HEAD
  git -C $repoDir status --short
  Write-Host "checked-out SHA: $checkedOutSha"

记录仓库 URL、分支和 `$checkedOutSha`。`d7d4a16` 只是本次任务所知的参考基线，
不是必须写死或必须 checkout 的构建 SHA；以本次 `git rev-parse HEAD` 的证据为准。

## Windows 和工具链先决条件

建议使用 x64 Windows 10/11 的 Visual Studio 2022 Developer PowerShell 或 x64
Native Tools PowerShell，使 `cl` 和 `sh` 在同一会话中可用。先执行并记录每条
命令的完整命令、退出码和版本/结果：

  Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,OSArchitecture
  $env:PROCESSOR_ARCHITECTURE
  node --version
  npm --version
  git --version
  where.exe git
  where.exe sh
  sh --version
  rustc -Vv
  cargo -V
  rustup show active-toolchain
  rustup target list --installed
  where.exe cl
  cl 2>&1 | Select-Object -First 3

Node 使用仓库 `desktop/package.json` 的范围 `^20.19.0 || >=22.12.0`；本次优先使用
Node 22 LTS（>=22.12.0）。Rust 必须是 stable MSVC 工具链；需能使用
`x86_64-pc-windows-msvc`。`cl` 必须来自 Visual Studio 2022 的 Desktop
development with C++，并安装 MSVC v143 和 Windows 10/11 SDK。`sh` 必须是 Git
for Windows 提供的 shell，因为 Tauri 配置会在构建前调用 `sh scripts/with-project-npm.sh`。
缺少任一项时不要静默安装系统级前置条件，报告 BLOCKED、缺失项和最小安装建议后停止。

检查 WebView2 Runtime（只读注册表检查，不修改注册表）：

  $edgeRoots = @(
    'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients',
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients',
    'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients'
  )
  $webview2 = foreach ($root in $edgeRoots) {
    Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue |
      ForEach-Object { Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue } |
      Where-Object { $_.name -like '*WebView2*' } |
      Select-Object name,pv,PSPath
  }
  $webview2

必须能看到 Microsoft Edge WebView2 Runtime 及其版本；否则报告 BLOCKED。不要把
WebView2 Runtime 下载误报成应用运行时联网；Tauri 配置的 Windows bundle 使用
`offlineInstaller` 模式。

## 建立完全合成的手工复测 fixture

不要把 fixture 建在仓库或任何临床目录。以下 PowerShell 只在 `%TEMP%` 下生成一个
带时间戳的新目录，不删除已有目录：

  $fixtureRoot = Join-Path $env:TEMP ('GWP-SYNTHETIC-STAGE3-' + (Get-Date -Format yyyyMMddHHmmss))
  $caseA = Join-Path (Join-Path $fixtureRoot 'CAMPUS-A') '2026-02-03_CASE-ALPHA'
  $caseB = Join-Path (Join-Path $fixtureRoot 'CAMPUS-B') '2026-02-02_CASE-BETA'
  New-Item -ItemType Directory -Force -Path $caseA,$caseB | Out-Null

  $jpegBase64 = '/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////2wBDAf//////////////////////////////////////////////////////////////////////////////////////wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAX/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oADAMBAAIQAxAAAAF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABBQJ//8QAFBEBAAAAAAAAAAAAAAAAAAAAAP/aAAgBAwEBPwF//8QAFBEBAAAAAAAAAAAAAAAAAAAAAP/aAAgBAgEBPwF//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQAGPwJ//8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPyF//9k='
  $jpegBytes = [Convert]::FromBase64String($jpegBase64)
  foreach ($name in @('ASSET-01.JPG','ASSET-02.JPG','ASSET-03.JPG')) {
    [IO.File]::WriteAllBytes((Join-Path $caseA $name), $jpegBytes)
  }
  [IO.File]::WriteAllBytes(
    (Join-Path $caseA 'CLIP-01.MP4'),
    [Text.Encoding]::ASCII.GetBytes('SYNTHETIC-MP4-PLACEHOLDER')
  )
  [IO.File]::WriteAllBytes((Join-Path $caseB 'ASSET-01.JPG'), $jpegBytes)

  $templatePath = Join-Path $fixtureRoot 'label.txt'
  @(
    '*FIELD_ALPHA=OPTION_ONE|OPTION_TWO'
    'FIELD_BETA=OPTION_X|OPTION_Y'
  ) | Set-Content -LiteralPath $templatePath -Encoding ASCII
  Get-ChildItem -LiteralPath $fixtureRoot -Recurse -File | Select-Object FullName,Length

这个 fixture 正好有两个院区；`CASE-ALPHA` 有三张合成图片和一个 MP4 占位文件，
可验证两张上限和 MP4 限制；`CASE-BETA` 用于院区/病例切换。模板没有医学词汇，
第一字段是合成必填字段，第二字段是合成可选字段。记录 fixture 路径，但不要把
真实数据路径写入报告。

## 前端安装、测试和构建

  Set-Location (Join-Path $repoDir 'desktop')
  $env:npm_config_cache = Join-Path (Get-Location) '.npm-cache'
  npm ci
  $npmCiExit = $LASTEXITCODE
  if ($npmCiExit -ne 0) { Write-Host "npm ci exit=$npmCiExit"; exit $npmCiExit }

分别运行以下命令；每条命令之后记录 `$LASTEXITCODE`，不要只记录最后一条命令的结果：

  npm run check:node
  npm run test
  npm run build
  npm run smoke

记录前端测试输出中的通过/失败/跳过总数。`npm run smoke` 必须在 `npm run build`
之后运行，并应验证构建前端的挂载点、CSP 和无外部网络 URL。任一非零退出码都要
记录第一条错误并停止后续验收；不要用改配置的方式绕过检查。

## Cargo 检查

先确认使用仓库已提交的 `desktop\src-tauri\.cargo\config.toml`（当前 crates.io
替换为 `rsproxy-sparse` 的 sparse registry），不要编辑它或另加全局 Cargo 配置。
在 `desktop\src-tauri` 目录逐条执行：

  Set-Location (Join-Path $repoDir 'desktop\src-tauri')
  cargo fmt -- --check
  cargo clippy --all-targets --locked -- -D warnings
  cargo test --locked
  cargo build --locked

每条 Cargo 命令都记录命令、退出码和结果；记录 `cargo test` 的测试总数、通过数、
失败数和忽略数（以实际输出为准）。

如果任一 Cargo 命令第一次出现 registry/index/crate 下载或 TLS/网络访问失败：

1. 立即保存终端中第一条完整错误文本、失败命令和退出码；不要继续执行其他 Cargo
   命令，也不要反复重试来掩盖首次失败。
2. 不得使用 `-k`、关闭证书校验、关闭证书吊销检查、修改 Schannel/CRL、改写
   `HTTP_PROXY`/`HTTPS_PROXY` 或 WinHTTP/系统代理，不得替换仓库镜像配置。
3. 状态应为 BLOCKED；最小下一步只是在网络/仓库管理员允许的范围内核对当前
   `rsproxy.cn` registry 可达性和证书链，保留原错误后再决定是否重跑同一命令。

## 构建 NSIS 安装包

回到 `desktop` 后使用仓库内的 Tauri CLI 生成 Windows NSIS 包：

  Set-Location (Join-Path $repoDir 'desktop')
  npx --no-install tauri build --bundles nsis
  $tauriExit = $LASTEXITCODE
  Write-Host "tauri build exit=$tauriExit"

不要使用 `npm run tauri:package`，它不是本次 Windows NSIS 验收命令。如果构建因
Windows shell wrapper、MSVC、WebView2 或其他 Windows 特有问题失败，先保存第一
条完整错误；不要改源码或做宽泛重构。

安装包应位于：

  src-tauri\target\release\bundle\nsis\

从该目录选择本次实际生成的最新 `*setup.exe`，并记录完整路径、字节大小和 SHA-256：

  $nsisDir = Join-Path $repoDir 'desktop\src-tauri\target\release\bundle\nsis'
  $installer = Get-ChildItem -LiteralPath $nsisDir -Filter '*setup.exe' -File |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1
  if (-not $installer) { Write-Error '没有找到 NSIS setup.exe'; exit 3 }
  $installer.FullName
  $installer.Length
  Get-FileHash -LiteralPath $installer.FullName -Algorithm SHA256

## 安装、启动和合成 UI 验证

优先使用标准交互式安装器：

  $installerProcess = Start-Process -FilePath $installer.FullName -Wait -PassThru
  $installerProcess.ExitCode
  if ($installerProcess.ExitCode -ne 0) {
    Write-Error "安装器失败，退出码：$($installerProcess.ExitCode)"
    exit $installerProcess.ExitCode
  }

如确需静默安装，才使用 `/S`，并在报告中明确写“本次使用 NSIS 静默安装”；不要
把未记录安装方式的结果当作完整 PASS。安装失败时记录安装器退出码和第一条错误。

记录安装器实际使用的安装目录，并找到实际的 `GWP Annotator.exe`；不要假定固定
的用户名或固定安装路径。可在常见安装根目录内只读查找：

  $installRoots = @(
    (Join-Path $env:LOCALAPPDATA 'Programs'),
    $env:ProgramFiles,
    ${env:ProgramFiles(x86)}
  ) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }
  $installedExe = foreach ($root in $installRoots) {
    Get-ChildItem -LiteralPath $root -Filter 'GWP Annotator.exe' -File -Recurse -ErrorAction SilentlyContinue
  } | Select-Object -First 1
  if (-not $installedExe) { Write-Error '未找到已安装的 GWP Annotator.exe'; exit 4 }
  $installedExe.FullName

启动并记录可见窗口结果：

  $appProcess = Start-Process -FilePath $installedExe.FullName -PassThru
  Start-Sleep -Seconds 5
  $appProcess.Refresh()
  Get-Process -Id $appProcess.Id -ErrorAction SilentlyContinue |
    Select-Object Id,ProcessName,Responding,MainWindowTitle,MainWindowHandle

手工确认窗口没有立即退出，并且窗口标题/页面抬头为 `GWP Annotator`。未选择根目录
时，源码当前还应可观察到 `本地医生标注工作台`、`选择数据根目录`、`本地离线`、
`请选择本地数据根目录。` 和 `标签模板`；模板未加载提示为
`未加载。请先选择 JSON 或 label.txt 模板。`。不要把这些源码已给出的文本替换成
自创的 UI 名称。

在应用的“选择数据根目录”中选择 `$fixtureRoot`，然后按下列顺序逐项记录观察结果：

1. 看到 `已扫描 2 个院区。`，可在“院区”下拉框切换 `CAMPUS-A`/`CAMPUS-B`，
   并在病例列表切换两个合成病例；报告中不得写入真实姓名。
2. 进入 `CASE-ALPHA`，用“图片 01/02/03”和左右导航切换三张合成图片；切换到
   媒体序列末尾的 `MP4 04`（fixture 源文件名为 `CLIP-01.MP4`），确认它显示 MP4
   和当前不可播放/不可截帧/不可标注的延后提示。
3. 在两张图片上分别点击 `设为代表`。在第三张图片再点击 `设为代表`，确认前两张
   仍是代表图，并观察当前错误提示 `每个病例最多选择两张代表图。`。不要尝试把
   MP4 设为代表图。
4. 点击 `加载模板`，选择 `$templatePath`，保持模板 ID 为 `template-local`、版本
   为 `1`；确认模板条显示对应的 `template-local v1`，且字段来自合成模板而非内置
   医学词表。
5. 回到一张代表图片，确认状态为可圈选。按住鼠标左键在图像内画一个有面积的
   简单闭合轮廓，仍按住左键回到起点附近；确认出现 `填写区域标签` 对话框。必填
   `FIELD_ALPHA` 未完成时，OK 应保持禁用；选一个合成选项后点击 OK，确认出现
   `区域与标签已保存。`，区域侧栏出现 `区域 01`，数量为 `1 个区域`。
6. 再画一个轮廓并在对话框中点击 `取消`，确认出现 `已丢弃未保存的圈选。` 且
   数量仍为 `1 个区域`。再画一个轮廓，在对话框中按 `Esc`，确认同样丢弃且数量
   不增加。若点击 OK 实际未保存，或 Cancel/Esc 误保存，记录实际行为和第一处
   失败，不得自行修正源码后继续宣称 PASS。
7. 切换到另一张图片或 `CASE-BETA`，再切回已保存的合成图片，确认已保存区域列表
   仍可读出 `区域 01`；只验证当前进程内的持久化回读，不要求完全离线重启。

完成后可正常关闭应用。不要为了本次验收拔网线、禁用网络或做“完全离线重启”。
完全离线重启是可选观察项，不是验收门槛；运行时连接观察也不是本次要求，不要把
未执行它记为失败。任何自愿执行的重启或网络观察都必须在报告中标为“可选”，不能
改变 PASS/PARTIAL/BLOCKED 的门槛。

## 网络、安全和结果判定

依赖下载只允许发生在 clone、`npm ci`、Cargo 检查和构建阶段；这些下载不证明应用
运行时联网。始终保持 TLS 证书校验和证书吊销检查开启，绝不修改 Schannel/CRL、
系统或全局代理；不得通过环境变量、Git/Cargo/npm 全局配置绕过这些边界。应用启动
时的 `本地离线` 只按当前 UI 观察记录，不延伸承诺未实现的功能。

- PASS：先决条件齐全；前端命令、Cargo 命令和 NSIS 命令均成功；安装成功；能找到
  并启动可见的 `GWP Annotator.exe`；上述合成 UI 验证全部通过；没有访问真实患者
  媒体。完全离线重启和运行时连接观察无需执行。
- PARTIAL：已完成的检查和证据明确，但有一个不阻断安装/启动的 Stage 3 交互项
  未完成或行为与预期不符；必须列出第一处失败和可复现证据。
- BLOCKED：缺少先决条件、工作树不干净、依赖 registry 首次访问失败、前端/Cargo/
  NSIS 构建失败、安装失败、找不到安装 EXE 或程序不能显示窗口；必须在第一处阻断
  后停止，并给出最小安全下一步。

## 最终中文证据报告模板

不要打印秘密，不要粘贴真实患者信息。报告必须包含：

  总体：PASS / PARTIAL / BLOCKED
  仓库 URL：
  分支：
  checked-out SHA（来自 git rev-parse HEAD）：
  Windows 版本和架构：
  Node/npm/Rust/Cargo/MSVC/Git/sh/WebView2 版本：

  命令证据（每行一条）：
  - 命令：... | 退出码：... | 结果：...

  前端测试总数：... passed / ... failed / ... skipped
  Cargo 测试总数：... passed / ... failed / ... ignored
  合成 fixture：仅合成/仓库 fixture；真实患者媒体访问：否
  fixture 路径（仅合成路径）：

  NSIS 构建命令和退出码：
  安装器完整路径：
  安装器大小（bytes）：
  安装器 SHA-256：
  安装方式（交互式或明确写静默 `/S`）：
  安装结果：
  已安装 EXE 完整路径：
  启动结果/可见窗口标题：
  Stage 3 合成 UI 逐项结果：目录/院区/病例、图片导航、两张代表图上限、自由圈
  选自动闭合、模板/OK、Cancel、Esc、已保存区域回读、MP4 当前限制
  可选离线重启/运行时连接观察：未执行，或写清实际观察（不计入门槛）

  若不是 PASS：第一阻断/第一失败的完整错误、命令和退出码：
  最小安全下一步：

macOS 上的构建、测试或启动只能作为其他平台信息，不能证明 Windows 成功，也不能
替代本报告的 Windows 证据。
```
