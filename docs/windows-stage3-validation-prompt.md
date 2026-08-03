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

## 仅限合成 fixture

在 `%TEMP%` 下创建一个新目录，一级目录只能是 `CAMPUS-A` 与 `CAMPUS-B`，病例名称和媒体名称只使用 `CASE-...`、`ASSET-...` 等占位符。`CAMPUS-A` 的一个病例放入至少三张非临床、合成 JPG；为两个院区各放入至少一个病例。使用外部 JSON 或 `label.txt` 合成模板，不填写或推测医学标签词表。

不要把 fixture 放入仓库、项目工作目录或任何患者目录。不要把 fixture、SQLite、安装包、`node_modules`、`target` 或截图加入 Git。

## 本机检查

1. 依照仓库当前说明完成前端、Rust 检查和 Windows NSIS 构建。Windows 安装与运行结果只代表 Windows 手工复测；不要把 macOS 构建结果当作 Windows 通过。
2. 启动应用，选择合成根目录，确认两个院区、病例与图片导航可用，并加载合成标签模板。
3. 在同一病例连续把至少三张图片设为代表图。三张都应保持已选择，不能出现“最多两张”或“第三张被拒绝”的提示。
4. 打开其中一张代表图，将鼠标移到图片画布上。应出现由应用绘制的、高对比度白色/深色描边十字准星，而不是依赖硬件鼠标在截图中可见。将鼠标离开画布、切换图片、打开标签弹窗或打开“拖动平移”后，准星应立即消失；平移模式应保持 grab/drag 行为。未设代表图或未加载模板时不得显示可圈选准星。
5. 在已选代表图上完成一次有效圈选并以合成模板保存区域。尝试取消这张代表图时，应被拒绝；没有已保存区域的代表图仍可取消。确认未选代表图无法进入圈选。

## 报告

报告使用 `PASS`、`PARTIAL` 或 `BLOCKED`，并记录：实际 main SHA、使用的合成 fixture、检查/构建命令及退出码、安装与启动结果、至少三张代表图证据、十字准星状态检查、已保存区域禁止取消的结果，以及任何原始错误。明确写明“未访问真实患者媒体”。

本复测不要求断网，不要求修改代理或任何 TLS 设置，也不扩展或验收视频播放、截帧、训练导出、云服务、账号、模型或医学诊断功能。
