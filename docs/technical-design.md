# 胃癌白斑标注 V0.1 技术设计

## 1. 设计结论

推荐实现为 **Rust + Tauri 2 + TypeScript 前端（原生 DOM/Canvas 2D）+ SQLite** 的单机 Windows 桌面应用：Rust 负责目录扫描、ID 脱敏、SQLite、原子写入和导出；Tauri 使用 Windows WebView2 承载轻量界面；前端使用 HTML `video`、`CanvasRenderingContext2D` 和 Pointer Events 显示媒体、绘制覆盖层及实现自由圈选。

此选择优先满足 V0.1 的离线、轻量、启动快、浏览快和本地文件访问需求。它不引入本地模型、GPU、服务端、账号或网络依赖。

### 已确认范围

- Windows 单机，默认离线；仅服务胃癌术中白斑区域标注。
- 两院区目录、病例选择、左右键媒体浏览、按需数量的代表图、自由圈选、多区域、模板标签、MP4 播放和当前帧截取。
- 原始 JPG/MP4 永不改写；所有输出只使用脱敏病例 ID。
- 不含模型预测、分期、强制双人标注、审核、登录、云服务或自动医疗诊断。

### 后续可选功能

模板可视化编辑、区域编辑/撤销、更多编码兼容、筛选、双人标注、审核、账户、同步和模型辅助均留待后续版本；它们不改变本设计中的本地标注数据格式。

### 唯一待确认问题

首版模板中“白斑形状”和“置信度”的具体选项与必填规则尚未给定。实现应提供模板校验和导入，但不得内置推测出的医学词表。

## 2. 架构与职责

```text
Tauri WebView2 界面
  病例侧栏 / 媒体查看器 / Canvas 覆盖层 / 标签窗口
          | 受限 IPC（仅脱敏 ID 和用户操作）
Rust 核心
  扫描器 / 会话内路径映射 / 脱敏 ID / SQLite 仓储 / 原子文件写入 / 导出器
          |
      只读数据根目录                 应用项目工作目录（可写）
  两个院区 -> 病例目录 -> JPG/MP4    SQLite / 截帧 / 缩略图 / 恢复 / 导出
```

| 层 | 选型 | 责任 |
| --- | --- | --- |
| 桌面壳 | Tauri 2、Rust stable、WebView2 | 小体积 Windows 安装包、受控本地 IPC、无 Node 运行时常驻。安装器先检查 WebView2 Runtime；缺失时使用随离线安装介质提供的运行时，不在应用运行时联网下载。 |
| 前端 | TypeScript、原生 DOM、Canvas 2D、Pointer Events | 虚拟化病例侧栏、键盘导航、图像缩放平移、自由圈选与模板小窗口。避免为小界面引入重量级组件库。 |
| 图像 | `<img>` 或 `createImageBitmap` + 高 DPI Canvas 覆盖层 | 仅在选中时解码 JPG；底图和覆盖层使用同一变换矩阵，几何不随缩放、平移或设备像素比漂移。 |
| 视频 | WebView2 HTML `<video>` | 直接播放本地 MP4；当前帧画入离屏 Canvas 后编码为派生 JPG。依赖 Windows 已安装的媒体解码能力，不转码、不修改源视频。 |
| 本地存储 | SQLite（WAL、`synchronous=FULL`）+ 项目目录中的派生文件 | 提交一个区域使用一个事务；WAL 处理崩溃恢复，项目目录承载截帧、缩略图、恢复状态与导出。 |
| 导出 | GWA-JSON v1、COCO 1.0 几何兼容 JSON + 脱敏图像副本 | 原始标注可审计地导出，训练数据使用脱敏文件名、无源路径、无 EXIF。 |

## 3. 为什么适合轻量 Windows 单机软件

- Tauri 的 Rust 后端可用 Windows 文件系统 API 并发读取目录元数据，不需启动 Node 服务或数据库服务，也不捆绑完整 Chromium。
- WebView2 已随多数 Windows 10/11 设备提供；缺失时由离线安装介质补齐，Canvas 和原生视频元素足以完成 V0.1 的显示与自由圈选。
- SQLite 是进程内单文件数据库，无需安装服务、网络端口或账号；WAL 能将每次 `OK` 的短事务与界面浏览解耦。
- 目录索引、缩略图和媒体解码分层：扫描阶段不解码媒体，病例列表可先显示，缩略图按需生成并以不含姓名的 `asset_id` 缓存。

## 4. 被否决的主要替代方案

| 方案 | 不作为 V0.1 主方案的原因 |
| --- | --- |
| Electron + React | 可实现，但捆绑 Chromium 和 Node 运行时，安装体积、内存占用与冷启动不符合本项目优先级。 |
| 浏览器网页 + 本地 HTTP 服务 | 浏览器权限、端口、服务进程和本地路径访问会增加部署与隐私边界；离线单机体验较差。 |
| Python + Qt | 原型快，但冻结包体、媒体编解码一致性和长期 Windows 分发维护成本更高；目录扫描与原子存储也不如 Rust 路线直接。 |
| Flutter | UI 跨平台能力不是当前需求，Windows 本地视频与自由圈选仍需额外插件和较大运行时。 |
| 纯 Rust `egui`/`iced` | 图像可做，但本地 MP4 播放与常见 Windows 编码兼容需要额外媒体管线，增加 V0.1 风险。 |
| 服务端、网页协作或云标注 | 与默认离线、无登录、无云服务和不外传病例路径的确认边界冲突。 |

## 5. 数据根目录扫描、排序与媒体浏览

### 5.1 扫描规则

1. 用户显式选择数据根目录；应用要求其第一层恰有两个可用院区目录，并在界面中显示院区别名。
2. 每个院区第一层子目录视为一个病例。目录名按第一个下划线切为日期前缀和仅供本机显示的原始显示名；不把显示名序列化到结果。
3. 仅枚举病例目录直接包含的 `.jpg`、`.jpeg`、`.mp4`，后缀大小写不敏感；不递归扫描、不改写源目录、不在扫描时读取像素或视频帧。
4. 解析 `yyyyMMdd`、`yyyy-MM-dd`、`yyyy.MM.dd` 日期前缀；可解析项按日期降序，无法解析项排在其后并按目录修改时间降序。相同键按 `case_id` 稳定排序。
5. 同一病例的媒体按自然文件名顺序排列；左右键在该序列中切换。媒体路径和原始文件名只保留在当前进程的内存映射中。

扫描器应在工作线程产生可分页的脱敏索引，前端先渲染可见病例。缩略图在媒体首次可见时生成到项目工作目录，缓存键为 `asset_id`，缓存失效只删除应用自己的派生文件。

### 5.2 稳定脱敏 ID

每次扫描在内存中计算：

```text
case_id  = "CASE-" + Base32(HMAC-SHA-256(project_secret, campus_code + NUL + raw_case_folder_name))[0..15]
asset_id = "ASSET-" + Base32(HMAC-SHA-256(project_secret, case_id + NUL + raw_media_name + NUL + size + NUL + mtime))[0..15]
```

`project_secret` 是仅在本机 Windows Credential Manager 中按项目 ID 保存的秘密，不进入项目工作目录、SQLite 或训练导出。原始目录名和媒体名只作为内存中的 HMAC 输入，绝不写入 SQLite、JSON、日志、缓存名或导出文件。重新扫描同一目录可得到相同 ID；若原始文件改名、大小或修改时间变化，`asset_id` 改变并在界面要求人工确认是否重新标注。

## 6. 界面、渲染和自动闭合自由圈选

### 6.1 图像坐标与渲染

- 底图保留原始像素尺寸 `source_width`、`source_height`；覆盖层画布的后备缓冲区按 `devicePixelRatio` 设置。
- 前端使用同一个 `view_transform`（缩放、平移、旋转为 0）绘制底图和多边形。指针坐标先逆变换为图像坐标，再归一化为 `[0, 1]` 保存。
- 保存归一化顶点避免窗口大小、缩放和高 DPI 改变标注位置。导出时按源图宽高恢复像素坐标。

### 6.2 自由圈选状态机

| 状态 | 事件 | 行为 |
| --- | --- | --- |
| `Idle` | 左键单击图像内 | 记录起点，进入 `Drawing`。 |
| `Drawing` | 鼠标移动 | 距上一个采样点至少 2 个屏幕像素才追加轨迹点，实时绘制预览。 |
| `Drawing` | 指针回到起点附近 | 当已有至少 3 个点、轨迹长度有效且距起点不超过 `max(12 CSS px, 显示图像短边的 1.5%)` 时自动闭合，进入 `LabelDialog`。 |
| `LabelDialog` | `OK` | 先校验模板必填项，再原子提交区域和标签，回到 `Idle`。 |
| `LabelDialog` | `Cancel` 或 `Esc` | 丢弃本次轨迹和未确认标签，回到 `Idle`。 |

仅已选代表图可进入 `Drawing`。每次 `OK` 只创建一个区域；已有区域继续显示，因而一个代表图可保存多个区域。无效、多边形面积过小或自相交无法修复时应提示重画，不能写入不完整几何。

### 6.3 标签窗口

闭合后弹出小型模态窗口。窗口按模板的 `control` 渲染下拉、单选或多选控件；`required: true` 的字段没有有效选择时禁用 `OK` 并显示该字段错误。窗口只保存稳定字段 ID 和选项 ID，显示文字来自模板。

## 7. 视频策略

- MP4 在中央查看器中由 `<video>` 播放，作为浏览媒体参与左右键序列。
- “截取当前帧”从已解码的当前画面绘制到离屏 Canvas，并写入项目目录 `captures/<case_id>/FRAME-<opaque-id>.jpg`；先写临时文件，成功后原子重命名。
- 截帧不写入原始病例目录，不改动 MP4，也不支持逐帧追踪。截帧必须先被标为代表图，并与 JPG 共用同一代表图和圈选流程。
- 标注记录保存 `source_kind: "captured_frame"`、脱敏 `source_asset_id` 和 `source_timestamp_ms`，不保存源视频文件名或原始路径。
- 若 Windows 解码器不支持某个 MP4，界面显示不可播放原因并允许继续浏览其他媒体；V0.1 不在后台联网下载编解码器或转码源文件。

## 8. 本地工作目录与持久化

数据根目录是只读输入。Windows 工作目录跟随安装位置所在磁盘，并放在安装目录旁的独立文件夹中；例如程序安装在 `D:\Apps\GWP Annotator\` 时：

```text
D:/Apps/GWP Annotator Data/PROJECT-LOCAL/
  annotations.sqlite3
  captures/
    CASE-7Q2M8D4K/
      FRAME-ASSET-K9P3.jpg
  thumbnails/
    ASSET-N4Q2.webp
  recovery/
    session-state.json
  exports/
    export-20260802T103000Z/
      annotations.gwa.json
      dataset.coco.json
      images/
        CASE-7Q2M8D4K__ASSET-N4Q2.jpg
```

示例中的所有 ID 都是脱敏占位 ID。工作目录、导出目录和文件名不得含真实姓名、原始病例目录名或原始媒体路径。

### 8.1 SQLite 逻辑表

| 表 | 关键字段 | 说明 |
| --- | --- | --- |
| `cases` | `case_id`, `campus_code`, `representative_count`, `updated_at` | 不存储原始病例名或路径。 |
| `assets` | `asset_id`, `case_id`, `kind`, `width`, `height`, `source_fingerprint` | `kind` 为 `original_image` 或 `captured_frame`；指纹是不可逆的 HMAC 输出。 |
| `representative_assets` | `case_id`, `asset_id`, `selected_at` | 一个病例可按需保留任意数量的代表图。 |
| `regions` | `region_id`, `asset_id`, `polygon_norm_json`, `template_id`, `template_version`, `created_at` | 一个区域一行，顶点为归一化坐标。 |
| `region_label_values` | `region_id`, `field_id`, `value_json` | 单选/下拉保存一个 option ID；多选保存 option ID 数组。 |
| `templates` | `template_id`, `version`, `sha256`, `canonical_json` | 保存已使用模板的不可变快照，不含患者信息。 |

### 8.2 自动保存与崩溃恢复

1. `OK` 后在单个 SQLite 事务内写入 `regions`、标签值和更新时间，提交成功后才在界面显示“已保存”。数据库使用 WAL 和 `synchronous=FULL`。
2. 每次捕获帧先写 `*.tmp`，完成并校验后原子重命名；启动时清理未引用的临时文件，不触碰源目录。
3. 当前病例的脱敏 ID、当前媒体 ID、视图变换和未确认轨迹每 2 秒原子写入 `recovery/session-state.json`。异常退出后可恢复到该工作状态，但未点击 `OK` 的区域始终标为草稿，必须再次确认才会进入数据库。
4. 每次正常关闭和每 20 次提交后创建可轮换的 SQLite 备份快照。备份失败必须提示，不得影响已经成功提交的事务。

## 9. 标签模板格式

### 9.1 正式 TOML 模板

医生维护的正式格式为 `label-template-v1.toml`。医学负责人填入实际字段和选项；以下名称均为中性结构占位，不能作为默认医学词表发布。

```toml
format = "gwa-label-template"
version = 1
template_id = "template-local"

[[fields]]
id = "field_alpha"
label = "字段 A"
control = "single_select"
required = true

[[fields.options]]
id = "option_a"
label = "选项 A"

[[fields.options]]
id = "option_b"
label = "选项 B"

[[fields]]
id = "field_beta"
label = "字段 B"
control = "dropdown"
required = true

[[fields.options]]
id = "option_a"
label = "选项 A"

[[fields.options]]
id = "option_b"
label = "选项 B"
```

允许字段的 `control` 取值只有 `dropdown`、`single_select`、`multi_select`。`id` 和每个选项 `id` 在各自作用域必须唯一且稳定；`required = true` 的字段必须至少有一个选项。模板变更应产生新 `version`，旧标注仍由 SQLite 中存储的规范化 JSON 快照解释；该 JSON 是内部表示，不是医生日常维护格式。

### 9.2 简化 `label.txt` 导入

```text
# 仅为格式示例；没有内置医学选项
*字段 A=选项 A|选项 B
字段 B=选项 A|选项 B
```

规则：空行和 `#` 注释忽略；行首 `*` 表示必填；等号左侧是显示名；竖线分隔选项。导入器为每个字段和选项生成稳定 ID，控件默认 `dropdown`。TXT 仅用于首次兼容导入；需要单选、多选、显式 ID 或后续模板更换时使用 TOML。

## 10. 标注与训练导出格式

### 10.1 GWA-JSON v1 标注导出

`annotations.gwa.json` 是可读、可验证的标注交换格式。它不含原始路径、原始文件名或患者姓名：

```json
{
  "format": "gwa-annotation",
  "version": 1,
  "project_id": "PROJECT-01",
  "templates": [
    {
      "template_id": "template-local",
      "version": 1,
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "fields": [
        {
          "id": "field_alpha",
          "control": "single_select",
          "required": true,
          "options": ["option_a", "option_b"]
        },
        {
          "id": "field_beta",
          "control": "dropdown",
          "required": true,
          "options": ["option_a", "option_b"]
        }
      ]
    }
  ],
  "assets": [
    {
      "case_id": "CASE-7Q2M8D4K",
      "asset_id": "ASSET-N4Q2",
      "kind": "original_image",
      "width": 4032,
      "height": 3024,
      "representative": true,
      "regions": [
        {
          "region_id": "REGION-01",
          "template_id": "template-local",
          "template_version": 1,
          "polygon_norm": [[0.121, 0.220], [0.181, 0.241], [0.164, 0.301]],
          "labels": {
            "field_alpha": "option_a",
            "field_beta": "option_b"
          }
        }
      ]
    }
  ]
}
```

`templates` 包含所有被导出区域引用的模板版本、校验和与字段/选项 ID 快照；每个区域明确引用其保存时的模板版本。为避免把显示文本同错误的历史版本关联，生产导出还应保留从正式 TOML 生成的完整规范化模板快照；上例仅缩写显示文字以突出数据关系。`polygon_norm` 的点顺序为圈选顺序，坐标必须在 `[0, 1]` 内。`labels` 的键和值分别是模板字段 ID 和选项 ID；多选值为数组。缺少、越界或与模板不匹配的值必须导致导入/导出校验失败。

### 10.2 训练导出

训练导出生成：

- `dataset.coco.json`：COCO 1.0 的 `images`、`annotations`、`categories` 几何结构；唯一固定类别为中性的 `annotated_region`，模板字段值放在每个 `annotation.attributes` 扩展对象中，避免捏造医学类别。
- `images/CASE-...__ASSET-....jpg`：仅已选代表图的脱敏副本。导出器重编码图像以移除 EXIF，不改变原始 JPG，也不导出 MP4。
- `annotations.gwa.json`：与 SQLite 中已确认数据一致的原始结构化交换文件。
- `manifest.csv`：仅 `case_id`、`asset_id`、脱敏导出相对路径、宽高和媒体类型；不含原始路径、文件名或姓名。

COCO 多边形坐标在导出时由 `polygon_norm` 乘以源图像宽高得到。截帧也作为普通导出 JPG，保留其脱敏 `asset_id`，来源时间仅存于 GWA-JSON 的可选脱敏溯源字段中。

## 11. 安全、离线和失败处理

- 默认网络策略为零网络请求；不包含登录、云同步、远程遥测、在线模型或自动诊断。
- Tauri 文件访问白名单只授予用户选择的数据根目录的读取能力，以及应用项目工作目录的写入能力；禁止向数据根目录写入。
- 日志使用 `case_id`、`asset_id` 和错误码，禁止记录原始路径、目录名或媒体文件名。
- 导出前进行隐私字段扫描：发现路径分隔符、原始目录名缓存字段、EXIF 或非脱敏输出名即失败，不产生部分完成的导出目录。
- 无法读取、解码或写入时，保留已提交标注并报告具体操作（重新选择目录、检查写权限、跳过不支持媒体）；不以删除源文件或联网上传作为恢复手段。

## 12. 实施顺序

1. 建立 Rust 核心：项目工作目录、只读扫描、内存路径映射、HMAC ID、SQLite 迁移和原子保存。
2. 建立 Tauri 界面：院区/病例/媒体导航、按需缩略图、图片渲染和键盘焦点规则。
3. 实现自由圈选、模板解析与标签小窗口，使用假数据和占位模板测试所有控件。
4. 增加 MP4 播放与当前帧截取，确保截帧走同一代表图与标注管线。
5. 实现 GWA-JSON、COCO 和隐私扫描导出，再按验收标准在 Windows 参考设备上验证。
