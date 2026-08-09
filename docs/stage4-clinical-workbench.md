# Stage 4 临床工作台设计说明与验收记录

## 1. Design Read

Stage 4 面向 Windows 上执行临床影像标注的医生。工作台应当克制、紧凑、可信：把病例导航、影像勾画和已保存区域放在连续的工作面内，优先支持反复查看、选择、圈选和核对，而不是呈现营销式或装饰性的界面。工作台结构、控件和状态使用原生 DOM/CSS；既有的图像标注画布及其业务交互不在本阶段改写范围内。

本阶段的视觉参数为 `DESIGN_VARIANCE 3`、`MOTION_INTENSITY 2`、`VISUAL_DENSITY 7`：布局保持可预测且低变化，不增加自动动画，以紧凑但可读的信息密度支持重复操作。三栏工作区、工具栏语义和完整交互状态是由工作流需求确定的结构，不是对参数数字的重新解释。

## 2. 改前审计

审计基线是 Stage 4 前的同一工作台 DOM 与流程，关注以下问题并以保留式改版处理：

- **颜色、字体、圆角、间距**：原实现以分散的灰蓝值、`Segoe UI`/`Microsoft YaHei UI`、约 `15px` 基础字号和 `4px` 控件圆角为主；间距混用 `rem`。Stage 4 将常用表面、边框、文字、强调、危险和焦点色收敛为 CSS token，并采用较紧凑的 `14px`/`1.35` 基础排版、`3px` 控件圆角和稳定的小间距。
- **信息架构与控件层级**：项目操作、模板上下文、病例、媒体和区域已有明确任务归属，但需要更强的主区分隔和操作层级。Stage 4 保留原路径，明确为项目带、模板带、三栏工作区和区域内操作，不改变病例/媒体/区域的业务顺序。
- **选中、禁用、错误、空状态**：审计选择项、不可操作按钮、消息提示与无数据文案是否能独立识别。Stage 4 以蓝色左边界/浅色底区分选中项，统一禁用样式，给通知增加状态 hook，并保留“选择根目录后显示院区和病例”“从左侧选择病例”“尚未保存区域”等空状态。
- **缩放稳定性**：审计宽度、低高度和窄屏下工具条换行、三栏最小宽度、列表滚动与画布是否互相挤压。Stage 4 将画布容器声明为 `contained`，桌面端保持三栏；在 `1080px`、`900px`、`620px` 和低于 `760px` 高度的断点按既有内容约束收缩或改为单栏，不把图像缩放含义改为 CSS 布局含义。
- **AI 痕迹**：不使用渐变、装饰性大圆角卡片、宣传性文案、无任务含义的动效或夸张色块；仅保留功能性的焦点、选择、错误和模态层。

本节是代码与样式审计记录，不构成截图视觉结论或 Windows 真机结论。

## 3. Stage 4 视觉合约

### Token、形状与层级

- Token：`--surface-*` 区分应用、面板、控件、静默和选中表面；`--border`/`--border-strong` 区分边界；`--text-*` 区分正文和辅助文字；`--accent-*`、`--danger`、`--focus` 用于可操作、危险和键盘焦点。画布使用独立深色表面，避免与表单面混淆。
- 形状：常规控件为 `3px`，模态为 `4px`；列表行无圆角，借助边框与左侧选中标记组织连续信息。按钮保持稳定最小高度并禁止文本换行造成尺寸跳变。
- 层级：项目带最高，模板带提供当前上下文，三栏工作区承载任务；媒体区内部固定为“媒体工具条、画布、媒体条”。功能十字光标为 `z-index: 6`，模态为 `z-index: 100`，不会与基础内容竞争层级。

### 响应策略

桌面优先的网格使用最小宽度约束，确保病例列表、画布和检查器各自可滚动而不压缩画布容器。`<=1080px` 仅收紧列宽和控件内边距；`<=900px` 转为单列并使页面可滚动；`<=620px` 将页首、模板和视频控件改为可换行的垂直流；低高度桌面窗口缩小页边距及非核心条带高度。`prefers-reduced-motion` 下取消非必要动画。该策略用于布局稳定性，不修改原图像坐标、缩放、平移或已保存区域的语义。

## 4. 保留式改版边界

本阶段只调整工作台的视觉组织、CSS token 和前端可测 DOM 语义。以下内容明确不变：

- 病例/媒体浏览、代表图门槛、自由圈选、边界精修、标签编辑、保存、备份、视频截帧和恢复的业务规则。
- 原始媒体只读、本机显示名不持久化、脱敏 ID、离线本地工作流及既有隐私边界。
- 后端、SQLite schema、IPC、数据格式、模板格式、导出格式及依赖；本阶段没有后端、schema 或数据格式变更。
- 运行时不新增网络依赖；未访问、枚举、复制、散列或修改任何真实患者媒体、病例目录或安装数据。

## 5. DOM 合约与自动测试

新增的 hooks 供样式、自动化与回归测试稳定定位，不承载患者数据：

- 根节点：`data-stage="4"`、`data-visual-density="compact-clinical"`；项目和模板带分别标记 `data-workspace-zone="project-toolbar"`、`"template-toolbar"`。
- 主体：`data-workspace-layout="three-region"`，其直接子区依次使用 `case-navigation`、`media-canvas`、`annotation-inspector`。
- 媒体与状态：画布框为 `data-canvas-frame` 和 `data-canvas-size-contract="contained"`；媒体和区域项分别给出 `data-selected`，并保留 `aria-current`、`aria-pressed`；通知为 `data-status-kind`。
- 模态：遮罩为 `data-dialog-layer="modal"`，新建/编辑标签对话框为 `data-dialog-kind="create-region"`/`"edit-region"`，并保持 `aria-modal="true"`。

`desktop/src/main.test.ts` 的 Stage 4 回归测试在合成标注 API 上检查三主区、媒体区的三段固定顺序、选中语义、首尾媒体按钮禁用、画布尺寸合约，以及编辑标签模态的 hook 和无障碍属性。该测试不替代真实 Windows 安装验证，也不验证截图像素或临床业务结果。

## 6. 验收命令清单

从 `desktop/` 目录运行：

```bash
sh scripts/with-project-npm.sh run check:node
sh scripts/with-project-npm.sh run test
sh scripts/with-project-npm.sh run build
sh scripts/with-project-npm.sh run smoke
```

文档范围检查从仓库根目录运行：

```bash
git diff --check -- docs/stage4-clinical-workbench.md
```

本记录只报告实际运行的 Stage 4 前端命令，不将 macOS 自动化结果表述为 Windows 真机验证。

2026-08-10 本地验收结果：`check:node` 退出码 0；`test` 退出码 0（2 个测试文件、71 个测试全部通过）；`build` 退出码 0；`smoke` 退出码 0；仓库根目录 `git diff --check` 退出码 0。

## 7. 合成截图与复核结论

以下截图均由内存合成 API 和合成图像生成，位于 Git 忽略的 Playwright 输出目录：

- `desktop/output/playwright/stage4-1440x900.png`
- `desktop/output/playwright/stage4-1280x720.png`
- `desktop/output/playwright/stage4-1024x768.png`
- `desktop/output/playwright/stage4-1024x768-dialog.png`
- `desktop/output/playwright/stage4-1024x768-crosshair.png`

阶段主智能体逐图复核结论如下：

- `1440x900`：中央媒体区为第一视觉焦点；病例、媒体和区域三栏边界明确，列表行和选中态可快速扫描。页面与视口均为 `1440x900`，画布框约 `841x629`，按钮无文本裁切。
- `1280x720`：顶部项目操作保持单行，三栏和媒体三段结构未漂移。页面与视口均为 `1280x720`，画布框约 `748x461`，按钮无文本裁切。
- `1024x768`：仍保持约 `182 / 605 / 209px` 的三栏布局；页面与视口均为 `1024x768`，画布框约 `603x500`，没有无限下落、页面滚动或控件重叠。
- `1024x768` 模态：编辑标签对话框约 `560x278`，完整位于视口内，`z-index: 100`，焦点落在对话框，无遮挡或文字裁切。
- `1024x768` 十字准星：合成代表图上悬停时为 `visible`、`opacity: 1`、`pointer-events: none`、`z-index: 6`，画布系统光标隐藏，功能准星清楚可见。

展开“数据保存位置”后，1024 宽页面仍无滚动，画布框高度约 `447px`，证明可选上下文带不会导致画布无限下落。浏览器控制台唯一错误是开发预览缺少 `favicon.ico` 的 404，不影响应用业务或构建产物。Windows 安装包和 Windows 真机交互未在本阶段验证；任何真实患者媒体均未访问。
