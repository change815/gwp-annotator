# Windows Stage 3.1 手工复测

本复测仅由 Windows 使用者手工完成，并且只验证当前 `main` 的 Stage 3.1 本地标注工作台。只使用全新的合成 fixture；不得打开、枚举、复制、哈希、上传、修改或截图真实患者媒体或其目录。

## 安全取得当前 main

在单独的非临床工作目录中操作，不复用可能有未提交修改的旧工作目录。先确认工作区干净，再只进行快进更新；出现非快进、认证、网络或 TLS 错误时停止并记录原始错误，不修改 Git 配置、代理、TLS、Schannel 或 CRL 设置。

```powershell
git status --short
git fetch origin main
git switch main
git pull --ff-only origin main
git rev-parse HEAD
git status --short
```

也可在新的空目录执行 `git clone --branch main --single-branch <authorized-repository-url>`。记录实际 commit SHA；不要为复测提交、推送或创建 PR。

### 旧克隆提示 `M desktop/src-tauri/Cargo.toml`

这表示 Git 检测到该文件的工作树内容与当前提交不同，并不表示需要修改 Cargo。先检查是否存在实际内容差异：

```powershell
git diff --ignore-space-at-eol -- desktop/src-tauri/Cargo.toml
```

若该命令没有输出，通常只是旧 Windows 克隆的行尾状态。可只恢复这一文件，并在本次命令中临时要求 LF；不改全局 Git 设置：

```powershell
git -c core.autocrlf=false -c core.eol=lf restore --source=HEAD --worktree -- desktop/src-tauri/Cargo.toml
git status --short
git switch main
git pull --ff-only origin main
```

若第一条差异检查有实际文本输出，停止更新并保留该文件，不要用 `reset`、`clean` 或覆盖命令丢弃未知改动。

## 仅限合成 fixture

在 `%TEMP%` 下创建两个新的合成根目录。第一个只有 `CAMPUS-ONE/CASE-001/`；第二个有 3 个或更多任意命名院区，其中至少一个为中文合成名，例如 `CAMPUS-ONE/CASE-001/`、`院区-乙/CASE-002/` 与 `CAMPUS-THREE/CASE-003/`。病例和媒体名称只使用 `CASE-...`、`ASSET-...` 等占位符。每个院区至少有一个病例，且 `CAMPUS-ONE` 的一个病例放入至少三张非临床、合成 JPG。使用外部严格 UTF-8 TOML 合成模板，不填写或推测医学标签词表；不得使用 JSON 作为正式模板。

用 PowerShell 生成模板时，不使用默认编码。优先使用只含 ASCII 的 TOML `\uXXXX` 转义；若需要写入实际中文字符，使用 `[System.IO.File]::WriteAllText($path, $content, [System.Text.UTF8Encoding]::new($false))` 显式写出 UTF-8 无 BOM 文件。准备五个合成 TOML：ASCII 转义的正常模板、含实际中文显示名的合法 UTF-8 模板、故意含非法 UTF-8 字节的模板、含 `U+FFFD` 的模板和含明显乱码的模板。后三类都必须被拒绝，且不得尝试 GBK 或本地代码页转换。

不要把 fixture 放入仓库、项目工作目录或任何患者目录。不要把 fixture、SQLite、安装包、`node_modules`、`target` 或截图加入 Git。

## 本机检查

1. 开发侧的前端、Rust 检查已经完成，Windows 端不重复运行 Clippy 或测试。只在最新 `main` 的 `desktop` 目录执行 `npx --no-install tauri build --bundles nsis`，安装刚生成的 NSIS 包并启动；若打包出现真实编译错误，再记录首个完整错误。Windows 安装与运行结果只代表 Windows 手工复测，不把 macOS 构建结果当作 Windows 通过。
2. 启动应用，先选择单院区根目录，再选择 3+ 院区根目录。确认两种布局均可浏览病例和图片，且 UI 原样显示 `院区-乙` 等本机目录名；不能出现“必须两个院区”或同类数量提示。空/无有效院区布局必须报错。
3. 依次导入 ASCII 转义模板和含实际中文显示名的合法 UTF-8 模板，确认均可正常加载并驱动标签窗口。尝试导入故意非 UTF-8、含 `U+FFFD` 和明显乱码的合成模板，确认都被拒绝并提示另存为 UTF-8；不得发生 GBK 猜测转换。
4. 在同一病例连续把至少三张图片设为代表图。三张都应保持已选择，不能出现“最多两张”或“第三张被拒绝”的提示。
5. 打开其中一张代表图，将鼠标移到图片画布上。应出现由应用绘制的、高对比度白色/深色描边十字准星，而不是依赖硬件鼠标在截图中可见。将鼠标离开画布、切换图片、打开标签弹窗或打开“拖动平移”后，准星应立即消失；平移模式应保持 grab/drag 行为。未设代表图或未加载模板时不得显示可圈选准星。
6. 在已选代表图上完成一次有效圈选并以合成模板保存区域。尝试取消这张代表图时，应被拒绝；没有已保存区域的代表图仍可取消。确认未选代表图无法进入圈选。检查 SQLite、备份、日志和导出不含 `CAMPUS-ONE`、`院区-乙` 等合成源院区显示名，并用 `git status --short` 确认 fixture 和运行产物没有被加入 Git。

## 报告

报告使用 `PASS`、`PARTIAL` 或 `BLOCKED`，并记录：实际 main SHA、单院区和 3+ 中文院区合成 fixture、模板编码五项检查、检查/构建命令及退出码、安装与启动结果、至少三张代表图证据、十字准星状态检查、已保存区域禁止取消的结果，以及任何原始错误。明确写明“未访问真实患者媒体”。

本复测不要求断网，不要求修改代理或任何 TLS 设置，也不扩展或验收视频播放、截帧、训练导出、云服务、账号、模型或医学诊断功能。
