# 胃癌白斑标注 V0.1 验收标准

## 1. 验收原则

- 只使用合成测试目录、占位模板和非临床图像/视频夹具；验收过程不得读取、复制或输出患者原始媒体。
- 所有结果、导出、日志断言和夹具名称只使用合成院区名与 `CASE-...`、`ASSET-...` 等脱敏 ID，不使用真实患者姓名或真实院区名。
- 除特别标注的人工体验检查外，每项应由自动测试或可重复命令验证。命令以下文约定的实现目录 `desktop/` 为准；架构阶段尚未创建该目录或应用代码。
- 任何“网络请求成功”“模型推理成功”均不是 V0.1 验收项；反而应验证默认运行时没有这类行为。
- 已确认：人工完全断网后重启不再是 V0.1 或 Stage 2.5 的验收门槛。该决定不引入登录、云同步、遥测或其他联网功能；运行时连接观察仍可作为补充证据。

## 2. 建议验收命令

实现完成后，从项目根目录运行：

```bash
rtk cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
rtk cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
rtk cargo test --manifest-path desktop/src-tauri/Cargo.toml
rtk npm --prefix desktop run test
rtk npm --prefix desktop run build
rtk npm --prefix desktop run e2e
```

UI 自动化应使用合成夹具根目录，并把网络出口拦截为失败。基准测试在 Windows 10/11、SSD、8 GB 以上内存的参考设备上运行三次，记录中位数。

## 3. 功能验收

| ID | 验收项 | 通过条件 | 验证方法 |
| --- | --- | --- | --- |
| AC-01 | Windows 本地启动 | 安装版应用可进入本机项目/目录选择界面；没有登录、云服务或模型下载提示。人工完全断网后重启不是通过条件。 | Windows 原生安装与启动验证；静态 CSP/依赖审查；可选的运行时连接观察。 |
| AC-02 | 一至多个院区扫描 | 合成根目录分别含 1 个院区及 3 个以上任意命名院区（其中至少一个为中文合成名）时，均可扫描和选择；UI 原样显示本机目录名，不因数量不是两个而报错。找不到任何“院区 -> 病例”两层结构或误选常见扁平院区目录时报告布局错误；任意嵌套树不作无法可靠实现的自动推断承诺。 | `file_scan_allows_one_chinese_named_campus`、`file_scan_allows_more_than_two_campuses`、`empty_campus_layout_is_rejected`、`selecting_a_flat_campus_directory_as_the_root_is_rejected` 与 Windows 合成复测。 |
| AC-03 | 病例文件夹约定 | 识别第一层病例目录及其直系 JPG/JPEG/MP4，后缀大小写不敏感；不递归读取其他目录。 | 合成树测试 `scan_direct_media_only`。 |
| AC-04 | 病例排序 | 可解析日期按降序；无效日期排后；排序键相同以 `case_id` 稳定排序。 | 参数化测试 `case_sorting_is_stable`。 |
| AC-05 | 媒体顺序和方向键 | 在病例中按自然文件名显示媒体；左/右键切换上一/下一项；标签窗口和文本控件获得焦点时不切换。 | Playwright/Tauri E2E `keyboard_media_navigation`。 |
| AC-06 | 懒扫描与浏览速度 | 参考夹具为 3 个院区、每例约 101 个病例、每例 10 JPG 和 2 MP4；首次元数据扫描中位数不超过 5 秒，病例列表首屏不等待缩略图，切换已缓存病例中位数不超过 300 ms。 | `scan_benchmark` 与人工录屏复核；夹具为非临床生成文件。 |
| AC-07 | 原始媒体只读 | 打开、浏览、圈选、截帧和导出前后，合成源 JPG/MP4 的内容散列、大小、修改时间和目录清单完全不变。 | `source_media_immutable` 集成测试。 |
| AC-08 | 代表图选择与标注门槛 | 医生可在一个病例内按需选择代表图，至少可连续选择 3 张；未选代表图不能标注或导出；已有已保存区域的代表图不能取消选择。 | 集成测试 `representative_selection_has_no_limit_and_keeps_annotation_gate` 与 `saved_regions_block_representative_deselection`。 |
| AC-09 | 图片显示坐标 | 在 100%、放大、平移及高 DPI 下，同一归一化多边形覆盖同一源图位置，导出像素坐标在误差 1 像素内。 | 渲染截图比较和 `normalized_geometry_roundtrip`。 |
| AC-10 | 自动闭合自由圈选 | 左键单击一次起点后即可松开；后续 `buttons=0` 的鼠标移动仍记录轨迹。满足至少 3 点、有效面积且回到定义阈值内时自动闭合并弹出标签窗口；圈选中的额外单击不重置草稿，`Esc` 丢弃且不保存。 | Pointer Events 前端测试与合成 E2E `lasso_auto_close`。 |
| AC-11 | 无效轨迹 | 面积过小、点数不足或无法修复的自相交轨迹不得写入数据库，界面要求重画。 | `reject_invalid_polygon` 单元/集成测试。 |
| AC-12 | 严格 UTF-8 TOML 与模板控件 | 严格 UTF-8 TOML 分别驱动下拉、单选、多选及必填校验；必填项未完成时 `OK` 不可用。含实际中文显示名的合法 UTF-8 模板正常加载；故意非 UTF-8 字节、`U+FFFD` 或明显乱码模板被拒绝并提示另存为 UTF-8，不猜测或转换 GBK。校验和一致的历史乱码模板可恢复并更换，但不能创建新区域或“删除后重画”。 | 前端 `template UTF-8 import` 测试、Rust `toml_template_controls_and_required_values_are_validated`、`template_parsers_reject_encoding_damage`、`historical_damaged_templates_can_be_verified_recovered_and_replaced` 与 Windows PowerShell 合成复测。 |
| AC-13 | 简化模板导入 | `label.txt` 的注释、必填星号和竖线选项可正确导入为下拉字段；重复字段/选项和空必填选项被拒绝。 | `label_txt_import_validation` 单元测试。 |
| AC-14 | 标签医学边界 | 安装包和默认模板不含推测出的“白斑形状”或“置信度”医学选项；仅使用由医学负责人提供的模板选项。 | 源码/资源审查和 `no_builtin_medical_vocabulary` 测试。 |
| AC-15 | `OK` 立即保存 | 一次有效 `OK` 在单一 SQLite 事务内生成区域和标签，界面只在提交成功后提示已保存。 | 故障注入测试 `save_is_atomic`。 |
| AC-16 | `Cancel` 丢弃 | 点击 `Cancel` 或 `Esc` 后，本次轨迹和未确认标签不存在于 SQLite、GWA-JSON 和导出。 | E2E `cancel_discards_draft`。 |
| AC-17 | 多区域 | 一张代表图能保存两个及以上独立区域，分别带有模板标签；它们重新打开后均可显示。 | `multiple_regions_per_asset` 集成测试。 |
| AC-18 | MP4 浏览 | 可播放的合成 MP4 在中央查看器可播放、暂停并随左右键参与浏览。 | E2E `mp4_browse`。 |
| AC-19 | 当前帧截取 | 截帧只写到应用工作目录，以脱敏名保存；其可被选为代表图并执行同一圈选与标签流程。 | `capture_frame_pipeline` 集成/E2E。 |
| AC-20 | 不做视频追踪 | 产品没有逐帧标注队列、跟踪按钮或后台视频分析任务；截帧后仅当作独立图像。 | UI/任务列表审查。 |

## 4. 数据、隐私和恢复验收

| ID | 验收项 | 通过条件 | 验证方法 |
| --- | --- | --- | --- |
| AC-21 | 脱敏 ID | 同一合成病例跨重启扫描得到相同 `CASE-...`；变更原始媒体名、大小或修改时间会产生新 `ASSET-...` 并提示重新确认。 | `opaque_id_stability` 测试。 |
| AC-22 | 不持久化显示名/路径 | SQLite、备份、GWA-JSON、COCO、CSV、缩略图文件名、恢复文件和日志均不含合成源院区/病例显示名、原始路径或路径分隔符字段；fixture 和运行产物不会被加入 Git。 | `privacy_leak_scan` 对运行产物执行字符串和 JSON 模式检查，并检查 `git status --short`。 |
| AC-23 | 本机显示边界 | 原始院区和病例目录显示名只在本机 UI 会话中可见；重新扫描后由源目录重建，SQLite、备份、日志、Git 和导出不保存该字段。 | 内存/数据库模式审查与 E2E。 |
| AC-24 | 截帧与缓存位置 | 截帧、缩略图、恢复文件、SQLite 和导出都在项目工作目录；源数据根目录中没有新增文件。Windows 安装在 D 盘时，工作目录位于安装目录旁的 `GWP Annotator Data` 且同样在 D 盘。 | `derived_files_outside_source_root`、工作目录计算单测及 Windows 合成安装复测。 |
| AC-25 | 崩溃恢复已保存数据 | 在 `OK` 提交或完成病例备份后关闭进程，重启后自动回到当前图片并加载区域和标签；界面报告加载数量并提供手工重新加载入口。 | `crash_after_commit_recovery`、`original_image_regions_reload_after_case_backup_and_service_recreation` 与前端冷启动回读测试。 |
| AC-26 | 草稿不误提交 | 在标签窗口中、未点 `OK` 时强制终止进程；重启可恢复草稿视图，但数据库和导出均无该区域，需再次确认才保存。 | `draft_requires_reconfirmation` E2E。 |
| AC-27 | 备份失败可见 | 模拟备份目标不可写时，已提交标注仍可读取，界面显示备份失败，不删除数据库或源媒体。 | `backup_failure_is_non_destructive` 测试。 |
| AC-28 | 模板版本可解释 | 当前 TOML 可在确认后更换；历史标注保存模板 ID、版本、校验和与快照，仍可按旧模板修改和解释。同一已使用 ID/version 的不同内容必须拒绝。 | `template_snapshot_roundtrip` 与 `controlled_template_replacement_preserves_historical_regions_and_snapshots` 测试。 |

## 5. 导出验收

| ID | 验收项 | 通过条件 | 验证方法 |
| --- | --- | --- | --- |
| AC-29 | GWA-JSON | 输出符合 `gwa-annotation` v1；只包含已选代表图、归一化多边形、模板字段 ID 和选项 ID；越界坐标或非法标签使导出失败。 | JSON Schema/自定义校验 `gwa_export_roundtrip`。 |
| AC-30 | COCO 几何 | `dataset.coco.json` 中的图像 ID、文件名和路径均脱敏；多边形由归一化坐标正确换算为像素坐标；模板值位于 `annotation.attributes`。 | `coco_geometry_and_attributes` 测试。 |
| AC-31 | 训练图像导出 | 仅导出代表 JPG/截帧，文件名为 `CASE-...__ASSET-....jpg`，无 EXIF；MP4 和未选图片不会进入导出。 | `training_export_selection_and_exif` 集成测试。 |
| AC-32 | 无部分导出 | 写入/隐私扫描失败时，临时导出目录被清理或标为失败，最终导出目录不出现不完整数据。 | `export_is_atomic_and_private` 故障注入测试。 |
| AC-33 | 训练清单 | `manifest.csv` 仅含脱敏病例/媒体 ID、脱敏相对路径、尺寸和媒体类型；不含患者名、原始路径和源文件名。 | CSV 字段断言与 `privacy_leak_scan`。 |

## 6. 明确排除项验收

| ID | 排除项 | 通过条件 | 验证方法 |
| --- | --- | --- | --- |
| AC-34 | 无模型预测或自动诊断 | 无预测按钮、推理任务、模型权重、诊断结论或自动建议输出。 | 安装包清单、UI 和运行任务审查。 |
| AC-35 | 无分期输入 | 模板示例、数据库固定字段、UI 和导出没有分期专用输入。外部模板可定义的通用字段不构成内置分期功能。 | 模式与 UI 审查。 |
| AC-36 | 无强制双人标注或审核 | 可由单位医生完成保存和导出；没有必须第二人签署或审核状态机。 | E2E 和数据库模式审查。 |
| AC-37 | 无登录或云服务 | 没有身份提供商、账户配置、远程同步、云存储或网络 API；不依赖远程服务完成本地标注流程。人工完全断网验证为可选项。 | 依赖清单、配置审查和可选的运行时连接观察。 |

## 7. 人工最终验收清单

1. 分别使用含 1 个院区、含 3 个以上院区（包括中文合成院区名）的合成测试根目录选择院区、病例和媒体；确认 UI 显示真实本机目录名、没有“两院区”数量错误，且空/无有效院区布局会报错。
2. 对按需选择的代表图分别圈选多个区域，确认模板弹窗、`OK`、`Cancel`、恢复和导出行为符合上述条目。
3. 打开源目录和项目工作目录，对比文件清单与散列，确认源媒体没有变化且所有派生文件都在工作目录。
4. 用文本搜索复核 SQLite、备份、日志和导出：只应出现脱敏 ID，不能出现合成源院区/病例名、原始路径或患者姓名；再用 `git status --short` 确认没有 fixture 或运行产物被加入 Git。加载合法中文 UTF-8 TOML 后，另以故意非 UTF-8、含 `U+FFFD` 和明显乱码的合成 TOML 复测拒绝提示；不得发生 GBK 猜测转换。
5. 可选：在断网环境重复完整流程，作为离线运行的补充观察；这不是当前版本的验收门槛。

只有 AC-01 至 AC-37 全部通过，且唯一待确认问题已由医学负责人提供正式首版模板后，才可声明 V0.1 可交付。
