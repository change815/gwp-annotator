import { invoke } from "@tauri-apps/api/core";

import "./style.css";

export type MediaKind = "image" | "video";

export interface PublicAsset {
  asset_id: string;
  media_kind: MediaKind;
  size_bytes: number;
}

export interface PublicCase {
  case_id: string;
  date: string | null;
  assets: PublicAsset[];
}

export interface PublicCampus {
  campus_code: string;
  case_count: number;
  cases: PublicCase[];
}

export interface PublicScan {
  campuses: PublicCampus[];
}

export interface LocalDisplayCase {
  case_id: string;
  date: string | null;
  display_label: string;
}

export interface LocalDisplayCampus {
  campus_code: string;
  display_name: string;
  cases: LocalDisplayCase[];
}

export interface LocalDisplayScan {
  campuses: LocalDisplayCampus[];
}

export interface PointNorm {
  x: number;
  y: number;
}

export interface SavedRegion {
  region_id: string;
  asset_id: string;
  polygon_norm: PointNorm[];
}

export interface TemplateOption {
  id: string;
  label: string;
}

export interface TemplateField {
  id: string;
  label: string;
  control: "dropdown" | "single_select" | "multi_select";
  required: boolean;
  options: TemplateOption[];
}

export interface LabelTemplate {
  format: "gwa-label-template";
  version: number;
  template_id: string;
  fields: TemplateField[];
}

export type LabelValue = string | string[];

interface ImageAssetData {
  asset_id: string;
  mime_type: string;
  data_base64: string;
}

interface RepresentativeSelection {
  case_id: string;
  asset_id: string;
  representative_count: number;
}

interface TemplateDescriptor {
  template_id: string;
  version: number;
  sha256: string;
  template: LabelTemplate;
}

interface AppApi {
  chooseAndScanRoot(): Promise<PublicScan | null>;
  currentLocalDisplayScan(): Promise<LocalDisplayScan>;
  selectRepresentative(caseId: string, assetId: string): Promise<RepresentativeSelection>;
  deselectRepresentative(caseId: string, assetId: string): Promise<RepresentativeSelection>;
  representativeAssetIds(caseId: string): Promise<string[]>;
  readImageAsset(caseId: string, assetId: string): Promise<ImageAssetData>;
  savedRegionsForAsset(caseId: string, assetId: string): Promise<SavedRegion[]>;
  importJsonTemplate(input: string): Promise<TemplateDescriptor>;
  importLabelTxt(input: string, templateId: string, version: number): Promise<TemplateDescriptor>;
  saveConfirmedRegion(request: SaveRegionRequest): Promise<void>;
}

interface SaveRegionRequest {
  region_id: string;
  asset_id: string;
  polygon_norm: PointNorm[];
  template_id: string;
  template_version: number;
  labels: Record<string, LabelValue>;
}

declare global {
  interface Window {
    __GWP_API__?: AppApi;
  }
}

const tauriApi: AppApi = {
  chooseAndScanRoot: () => invoke<PublicScan | null>("choose_and_scan_root"),
  currentLocalDisplayScan: () => invoke<LocalDisplayScan>("current_local_display_scan"),
  selectRepresentative: (caseId, assetId) =>
    invoke<RepresentativeSelection>("select_representative", { caseId, assetId }),
  deselectRepresentative: (caseId, assetId) =>
    invoke<RepresentativeSelection>("deselect_representative", { caseId, assetId }),
  representativeAssetIds: (caseId) =>
    invoke<string[]>("representative_asset_ids", { caseId }),
  readImageAsset: (caseId, assetId) =>
    invoke<ImageAssetData>("read_image_asset", { caseId, assetId }),
  savedRegionsForAsset: (caseId, assetId) =>
    invoke<SavedRegion[]>("saved_regions_for_asset", { caseId, assetId }),
  importJsonTemplate: (templateJson) =>
    invoke<TemplateDescriptor>("import_json_template", { templateJson }),
  importLabelTxt: (text, templateId, version) =>
    invoke<TemplateDescriptor>("import_label_txt", { text, templateId, version }),
  saveConfirmedRegion: async (request) => {
    await invoke("save_confirmed_region", { request });
  },
};

interface DisplayCase extends PublicCase {
  displayLabel: string;
}

interface DisplayCampus {
  campusCode: string;
  displayName: string;
  cases: DisplayCase[];
}

interface Transform {
  scale: number;
  x: number;
  y: number;
}

interface ViewState {
  zoom: number;
  panX: number;
  panY: number;
}

interface DraftRegion {
  points: PointNorm[];
}

export interface CrosshairState {
  pointerInCanvas: boolean;
  x: number;
  y: number;
}

interface State {
  campuses: DisplayCampus[];
  campusCode: string | null;
  caseId: string | null;
  assetIndex: number;
  representativeIds: Set<string>;
  regions: SavedRegion[];
  selectedRegionId: string | null;
  image: HTMLImageElement | null;
  imageAssetId: string | null;
  imageLoading: boolean;
  view: ViewState;
  panMode: boolean;
  drawing: DraftRegion | null;
  labelDraft: DraftRegion | null;
  labelValues: Record<string, LabelValue>;
  template: LabelTemplate | null;
  templateFileName: string | null;
  crosshair: CrosshairState;
  message: string;
  messageKind: "info" | "error" | "success";
}

const initialState = (): State => ({
  campuses: [],
  campusCode: null,
  caseId: null,
  assetIndex: 0,
  representativeIds: new Set(),
  regions: [],
  selectedRegionId: null,
  image: null,
  imageAssetId: null,
  imageLoading: false,
  view: { zoom: 1, panX: 0, panY: 0 },
  panMode: false,
  drawing: null,
  labelDraft: null,
  labelValues: {},
  template: null,
  templateFileName: null,
  crosshair: { pointerInCanvas: false, x: 0, y: 0 },
  message: "请选择本地数据根目录。",
  messageKind: "info",
});

export function autoCloseThreshold(displayedShortSide: number): number {
  return Math.max(12, displayedShortSide * 0.015);
}

export function polygonArea(points: PointNorm[]): number {
  if (points.length < 3) return 0;
  let twiceArea = 0;
  for (let index = 0; index < points.length; index += 1) {
    const next = points[(index + 1) % points.length];
    twiceArea += points[index].x * next.y - next.x * points[index].y;
  }
  return Math.abs(twiceArea) / 2;
}

export function canAutoClose(
  points: PointNorm[],
  distanceToStartCssPx: number,
  displayedShortSide: number,
): boolean {
  if (points.length < 3 || polygonArea(points) < 0.0001) return false;
  return distanceToStartCssPx <= autoCloseThreshold(displayedShortSide);
}

export function primaryDrawingButtonIsPressed(buttons: number): boolean {
  return (buttons & 1) === 1;
}

export function shouldHandleMediaNavigation(event: KeyboardEvent): boolean {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return false;
  if (event.defaultPrevented) return false;
  const target = event.target instanceof Element ? event.target : null;
  if (!target) return true;
  if (target.closest('[role="dialog"]')) return false;
  return !target.matches("input, textarea, select, [contenteditable='true']");
}

export function labelsMeetRequiredFields(
  fields: TemplateField[],
  values: Record<string, LabelValue>,
): boolean {
  return fields.every((field) => {
    if (!field.required) return true;
    const value = values[field.id];
    return Array.isArray(value) ? value.length > 0 : Boolean(value);
  });
}

export function canAnnotate(representative: boolean, templateLoaded: boolean): boolean {
  return representative && templateLoaded;
}

export function shouldShowCustomCrosshair(
  representative: boolean,
  templateLoaded: boolean,
  panMode: boolean,
  pointerInCanvas: boolean,
  imageReady: boolean,
  labelDialogOpen: boolean,
): boolean {
  return representative && templateLoaded && !panMode && pointerInCanvas && imageReady && !labelDialogOpen;
}

export function clearCrosshairPointerState(crosshair: CrosshairState): CrosshairState {
  return { ...crosshair, pointerInCanvas: false };
}

export function customCrosshairMarkup(visible: boolean, x: number, y: number): string {
  const safeX = Number.isFinite(x) ? Math.round(x) : 0;
  const safeY = Number.isFinite(y) ? Math.round(y) : 0;
  return `<div class="drawing-crosshair ${visible ? "visible" : ""}" data-drawing-crosshair="${visible ? "visible" : "hidden"}" aria-hidden="true" style="pointer-events: none; z-index: 6; --crosshair-x: ${safeX}px; --crosshair-y: ${safeY}px;"></div>`;
}

function getApi(): AppApi {
  return window.__GWP_API__ ?? tauriApi;
}

function errorCode(error: unknown): string {
  if (typeof error === "object" && error && "code" in error) {
    return String((error as { code: string }).code);
  }
  return "UNEXPECTED_ERROR";
}

function formatError(error: unknown): string {
  const code = errorCode(error);
  const messages: Record<string, string> = {
    INVALID_CAMPUS_LAYOUT: "数据根目录必须正好包含两个院区目录。",
    SOURCE_WORK_OVERLAP: "数据根目录与工作目录重叠，已拒绝操作。",
    REPRESENTATIVE_HAS_REGIONS: "该代表图已有保存区域，不能取消选择。",
    NOT_IMAGE_ASSET: "只有图片可以作为代表图。",
    ASSET_NOT_REPRESENTATIVE: "请先选择当前图片为代表图。",
    TEMPLATE_NOT_LOADED: "请先加载标签模板。",
    INVALID_LABELS: "请完成所有必填标签。",
    INVALID_POLYGON: "圈选无效，请重新绘制闭合区域。",
  };
  return messages[code] ?? `操作未完成：${code}`;
}

function mergeScans(publicScan: PublicScan, localScan: LocalDisplayScan): DisplayCampus[] {
  const localByCampus = new Map(
    localScan.campuses.map((campus) => [campus.campus_code, campus]),
  );
  return publicScan.campuses.map((campus) => {
    const localCampus = localByCampus.get(campus.campus_code);
    const localCases = new Map(
      (localCampus?.cases ?? []).map((item) => [item.case_id, item]),
    );
    return {
      campusCode: campus.campus_code,
      displayName: localCampus?.display_name ?? campus.campus_code,
      cases: campus.cases.map((item) => ({
        ...item,
        displayLabel: localCases.get(item.case_id)?.display_label ?? item.case_id,
      })),
    };
  });
}

function selectedCampus(state: State): DisplayCampus | null {
  return state.campuses.find((campus) => campus.campusCode === state.campusCode) ?? null;
}

function selectedCase(state: State): DisplayCase | null {
  return selectedCampus(state)?.cases.find((item) => item.case_id === state.caseId) ?? null;
}

function selectedAsset(state: State): PublicAsset | null {
  return selectedCase(state)?.assets[state.assetIndex] ?? null;
}

function assetLabel(asset: PublicAsset, index: number): string {
  const kind = asset.media_kind === "image" ? "图片" : "MP4";
  return `${kind} ${String(index + 1).padStart(2, "0")}`;
}

function randomRegionId(): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const values = new Uint8Array(16);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(values);
  } else {
    for (let index = 0; index < values.length; index += 1) {
      values[index] = Math.floor(Math.random() * 256);
    }
  }
  return `REGION-${Array.from(values, (value) => alphabet[value % alphabet.length]).join("")}`;
}

function imageFromPayload(payload: ImageAssetData): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("IMAGE_DECODE_FAILED"));
    image.src = `data:${payload.mime_type};base64,${payload.data_base64}`;
  });
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function controlMarkup(field: TemplateField, value: LabelValue | undefined): string {
  const required = field.required ? " <span class=\"required\">必填</span>" : "";
  if (field.control === "dropdown") {
    const options = [
      `<option value="">请选择</option>`,
      ...field.options.map(
        (option) =>
          `<option value="${escapeHtml(option.id)}" ${value === option.id ? "selected" : ""}>${escapeHtml(option.label)}</option>`,
      ),
    ].join("");
    return `<label class="field-label">${escapeHtml(field.label)}${required}<select data-label-field="${escapeHtml(field.id)}" aria-label="${escapeHtml(field.label)}">${options}</select></label>`;
  }
  if (field.control === "single_select") {
    return `<fieldset class="field-set"><legend>${escapeHtml(field.label)}${required}</legend>${field.options
      .map(
        (option) =>
          `<label><input type="radio" name="${escapeHtml(field.id)}" data-label-field="${escapeHtml(field.id)}" value="${escapeHtml(option.id)}" ${value === option.id ? "checked" : ""}>${escapeHtml(option.label)}</label>`,
      )
      .join("")}</fieldset>`;
  }
  const selected = Array.isArray(value) ? value : [];
  return `<fieldset class="field-set"><legend>${escapeHtml(field.label)}${required}</legend>${field.options
    .map(
      (option) =>
        `<label><input type="checkbox" data-label-field="${escapeHtml(field.id)}" value="${escapeHtml(option.id)}" ${selected.includes(option.id) ? "checked" : ""}>${escapeHtml(option.label)}</label>`,
    )
    .join("")}</fieldset>`;
}

function mountMarkup(state: State): string {
  const campus = selectedCampus(state);
  const currentCase = selectedCase(state);
  const asset = selectedAsset(state);
  const isImage = asset?.media_kind === "image";
  const isRepresentative = Boolean(asset && state.representativeIds.has(asset.asset_id));
  const annotationReady = canAnnotate(isRepresentative, Boolean(state.template));
  const imageReady = Boolean(state.image && state.imageAssetId === asset?.asset_id);
  const crosshairVisible = shouldShowCustomCrosshair(
    isRepresentative,
    Boolean(state.template),
    state.panMode,
    state.crosshair.pointerInCanvas,
    imageReady,
    Boolean(state.labelDraft),
  );
  const crosshairReady = annotationReady && !state.panMode && imageReady && !state.labelDraft;
  const media = currentCase?.assets ?? [];
  const currentIndex = asset ? state.assetIndex + 1 : 0;

  return `
    <main class="workbench" aria-labelledby="app-title">
      <header class="app-header">
        <div><h1 id="app-title">GWP Annotator</h1><p>本地医生标注工作台</p></div>
        <div class="header-actions">
          <button type="button" data-action="choose-root">选择数据根目录</button>
          <span class="offline-status" data-testid="offline-status">本地离线</span>
        </div>
      </header>
      <p class="notice ${state.messageKind}" role="status" aria-live="polite">${escapeHtml(state.message)}</p>
      <section class="template-strip" aria-label="标签模板">
        <strong>标签模板</strong>
        <span>${state.template ? `${escapeHtml(state.template.template_id)} v${state.template.version}` : "未加载。请先选择 JSON 或 label.txt 模板。"}</span>
        <label class="file-button">加载模板<input data-action="template-file" type="file" accept=".json,.txt,application/json,text/plain" aria-label="加载标签模板"></label>
        <label class="template-meta">模板 ID<input data-template-id value="template-local" maxlength="64" aria-label="label.txt 模板 ID"></label>
        <label class="template-meta">版本<input data-template-version type="number" min="1" value="1" aria-label="label.txt 模板版本"></label>
      </section>
      <div class="workspace-grid">
        <aside class="case-sidebar" aria-label="院区与病例">
          <label>院区<select data-action="campus" aria-label="切换院区" ${state.campuses.length === 0 ? "disabled" : ""}>
            ${state.campuses
              .map(
                (item) =>
                  `<option value="${escapeHtml(item.campusCode)}" ${item.campusCode === state.campusCode ? "selected" : ""}>${escapeHtml(item.displayName)} (${item.cases.length})</option>`,
              )
              .join("")}
          </select></label>
          <div class="case-list" role="listbox" aria-label="病例列表">
            ${(campus?.cases ?? [])
              .map(
                (item) =>
                  `<button type="button" role="option" aria-selected="${item.case_id === state.caseId}" class="case-item ${item.case_id === state.caseId ? "selected" : ""}" data-case-id="${escapeHtml(item.case_id)}"><span>${escapeHtml(item.displayLabel)}</span><small>${escapeHtml(item.date ?? "日期未识别")}</small></button>`,
              )
              .join("") || `<p class="empty-copy">选择根目录后显示院区和病例。</p>`}
          </div>
        </aside>
        <section class="media-panel" aria-label="媒体工作区">
          <div class="media-toolbar">
            <div><strong>${currentCase ? escapeHtml(currentCase.displayLabel) : "未选择病例"}</strong><span>${asset ? `${currentIndex} / ${media.length}` : ""}</span></div>
            <div class="media-actions">
              <button type="button" data-action="previous" aria-label="上一张" title="上一张" ${state.assetIndex <= 0 ? "disabled" : ""}>上一张</button>
              <button type="button" data-action="next" aria-label="下一张" title="下一张" ${state.assetIndex >= media.length - 1 ? "disabled" : ""}>下一张</button>
              <button type="button" data-action="fit" ${isImage ? "" : "disabled"}>适配</button>
              <button type="button" data-action="pan" aria-pressed="${state.panMode}" title="启用后左键拖动可平移图片" class="${state.panMode ? "active" : ""}" ${isImage ? "" : "disabled"}>${state.panMode ? "平移中" : "拖动平移"}</button>
            </div>
          </div>
          <div class="canvas-frame ${state.panMode ? "pan-mode" : ""} ${crosshairReady ? "crosshair-ready" : ""}" data-canvas-frame>
            ${!asset ? `<p class="empty-copy">从左侧选择病例。</p>` : ""}
            ${asset?.media_kind === "video" ? `<div class="video-deferred"><strong>MP4</strong><p>视频将在下一阶段处理。当前不可播放、不可截帧、不可标注。</p></div>` : ""}
            ${isImage ? `<canvas data-drawing-canvas aria-label="图片标注画布" tabindex="0"></canvas>` : ""}
            ${isImage ? customCrosshairMarkup(crosshairVisible, state.crosshair.x, state.crosshair.y) : ""}
            ${state.imageLoading ? `<div class="loading-cover">正在读取图片…</div>` : ""}
            ${isImage && !isRepresentative ? `<div class="canvas-hint">请先将当前图片选择为代表图，才可圈选。</div>` : ""}
            ${isImage && isRepresentative && !state.template ? `<div class="canvas-hint">请先加载标签模板，才可保存圈选。</div>` : ""}
            ${isImage && annotationReady ? `<div class="canvas-hint ready">左键开始自由圈选，回到起点附近自动闭合。</div>` : ""}
          </div>
          <div class="media-strip" aria-label="当前病例媒体序列">
            ${media
              .map((item, index) => {
                const representative = state.representativeIds.has(item.asset_id);
                const isCurrent = index === state.assetIndex;
                const toggle = item.media_kind === "image"
                  ? `<button type="button" class="representative-button" data-action="representative" data-asset-id="${escapeHtml(item.asset_id)}" ${representative && state.regions.some((region) => region.asset_id === item.asset_id) ? "title=\"已有保存区域，不能取消\"" : ""}>${representative ? "取消代表" : "设为代表"}</button>`
                  : `<span class="deferred-badge">下一阶段</span>`;
                return `<article class="media-item ${isCurrent ? "current" : ""}"><button type="button" data-action="asset" data-index="${index}" aria-current="${isCurrent ? "true" : "false"}">${assetLabel(item, index)}</button>${toggle}</article>`;
              })
              .join("")}
          </div>
        </section>
        <aside class="region-sidebar" aria-label="区域列表">
          <h2>已保存区域</h2>
          <p>${asset && isImage ? `${state.regions.length} 个区域` : "选择图片后查看"}</p>
          <div class="region-list">
            ${state.regions
              .map(
                (region, index) =>
                  `<button type="button" class="region-item ${region.region_id === state.selectedRegionId ? "selected" : ""}" data-region-id="${escapeHtml(region.region_id)}">区域 ${String(index + 1).padStart(2, "0")}</button>`,
              )
              .join("") || `<p class="empty-copy">尚未保存区域。</p>`}
          </div>
          <div class="annotation-state ${annotationReady ? "ready" : ""}">${isImage ? annotationReady ? "可圈选" : isRepresentative ? "等待模板" : "等待代表图" : "图片标注不可用"}</div>
        </aside>
      </div>
      ${state.labelDraft && state.template ? `
        <div class="modal-backdrop" data-action="cancel-draft">
          <section class="label-dialog" role="dialog" aria-modal="true" aria-labelledby="label-dialog-title" data-label-dialog tabindex="-1">
            <header><h2 id="label-dialog-title">填写区域标签</h2><button type="button" data-action="cancel-draft" aria-label="取消">取消</button></header>
            <p>区域尚未保存。完成必填项后才可确认。</p>
            <form data-label-form>
              ${state.template.fields.map((field) => controlMarkup(field, state.labelValues[field.id])).join("")}
              <div class="dialog-actions"><button type="button" data-action="cancel-draft">取消</button><button type="submit" ${labelsMeetRequiredFields(state.template.fields, state.labelValues) ? "" : "disabled"}>OK</button></div>
            </form>
          </section>
        </div>` : ""}
    </main>`;
}

function fitTransform(canvas: HTMLCanvasElement, image: HTMLImageElement, view: ViewState): Transform {
  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  const fit = Math.min(width / image.naturalWidth, height / image.naturalHeight);
  const scale = fit * view.zoom;
  return {
    scale,
    x: (width - image.naturalWidth * scale) / 2 + view.panX,
    y: (height - image.naturalHeight * scale) / 2 + view.panY,
  };
}

function normalizedFromPointer(pointer: PointNorm, transform: Transform, image: HTMLImageElement): PointNorm | null {
  const x = (pointer.x - transform.x) / (image.naturalWidth * transform.scale);
  const y = (pointer.y - transform.y) / (image.naturalHeight * transform.scale);
  if (x < 0 || x > 1 || y < 0 || y > 1) return null;
  return { x, y };
}

function canvasPoint(event: MouseEvent, canvas: HTMLCanvasElement): PointNorm {
  const bounds = canvas.getBoundingClientRect();
  return { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
}

function drawPolygon(
  context: CanvasRenderingContext2D,
  points: PointNorm[],
  transform: Transform,
  image: HTMLImageElement,
  color: string,
  active: boolean,
): void {
  if (points.length === 0) return;
  context.beginPath();
  points.forEach((point, index) => {
    const x = transform.x + point.x * image.naturalWidth * transform.scale;
    const y = transform.y + point.y * image.naturalHeight * transform.scale;
    if (index === 0) context.moveTo(x, y);
    else context.lineTo(x, y);
  });
  if (!active && points.length >= 3) context.closePath();
  context.lineWidth = active ? 2.5 : 2;
  context.strokeStyle = color;
  context.fillStyle = active ? "rgba(249, 115, 22, 0.12)" : "rgba(14, 116, 144, 0.12)";
  context.stroke();
  if (!active && points.length >= 3) context.fill();
}

function startApp(target: HTMLDivElement): void {
  const state = initialState();
  const api = getApi();
  let panOrigin: PointNorm | null = null;

  const message = (text: string, kind: State["messageKind"] = "info") => {
    state.message = text;
    state.messageKind = kind;
  };

  const refreshRegions = async (): Promise<void> => {
    const currentCase = selectedCase(state);
    const asset = selectedAsset(state);
    if (!currentCase || !asset || asset.media_kind !== "image") {
      state.regions = [];
      return;
    }
    state.regions = await api.savedRegionsForAsset(currentCase.case_id, asset.asset_id);
    if (!state.regions.some((region) => region.region_id === state.selectedRegionId)) {
      state.selectedRegionId = state.regions[0]?.region_id ?? null;
    }
  };

  const loadImage = async (): Promise<void> => {
    const currentCase = selectedCase(state);
    const asset = selectedAsset(state);
    if (!currentCase || !asset || asset.media_kind !== "image") {
      state.image = null;
      state.imageAssetId = null;
      state.imageLoading = false;
      render();
      return;
    }
    const requestedCase = currentCase.case_id;
    const requestedAsset = asset.asset_id;
    state.crosshair.pointerInCanvas = false;
    state.image = null;
    state.imageAssetId = null;
    state.imageLoading = true;
    render();
    try {
      const payload = await api.readImageAsset(requestedCase, requestedAsset);
      const image = await imageFromPayload(payload);
      if (selectedCase(state)?.case_id !== requestedCase || selectedAsset(state)?.asset_id !== requestedAsset) return;
      state.image = image;
      state.imageAssetId = requestedAsset;
      state.imageLoading = false;
    } catch (error) {
      if (selectedAsset(state)?.asset_id === requestedAsset) {
        state.imageLoading = false;
        message(formatError(error), "error");
      }
    }
    render();
  };

  const selectCase = async (caseId: string): Promise<void> => {
    const campus = selectedCampus(state);
    const nextCase = campus?.cases.find((item) => item.case_id === caseId);
    if (!nextCase) return;
    state.caseId = nextCase.case_id;
    state.assetIndex = 0;
    state.representativeIds = new Set(await api.representativeAssetIds(nextCase.case_id));
    state.regions = [];
    state.selectedRegionId = null;
    state.view = { zoom: 1, panX: 0, panY: 0 };
    state.drawing = null;
    state.crosshair.pointerInCanvas = false;
    await refreshRegions();
    await loadImage();
  };

  const selectAsset = async (index: number): Promise<void> => {
    const currentCase = selectedCase(state);
    if (!currentCase || index < 0 || index >= currentCase.assets.length) return;
    state.assetIndex = index;
    state.view = { zoom: 1, panX: 0, panY: 0 };
    state.drawing = null;
    state.labelDraft = null;
    state.crosshair.pointerInCanvas = false;
    await refreshRegions();
    await loadImage();
  };

  const draw = (canvas: HTMLCanvasElement): void => {
    const image = state.image;
    if (!image || state.imageAssetId !== selectedAsset(state)?.asset_id) return;
    const dpr = window.devicePixelRatio || 1;
    const width = Math.max(1, canvas.clientWidth);
    const height = Math.max(1, canvas.clientHeight);
    if (canvas.width !== Math.floor(width * dpr) || canvas.height !== Math.floor(height * dpr)) {
      canvas.width = Math.floor(width * dpr);
      canvas.height = Math.floor(height * dpr);
    }
    const context = canvas.getContext("2d");
    if (!context) return;
    context.setTransform(dpr, 0, 0, dpr, 0, 0);
    context.clearRect(0, 0, width, height);
    const transform = fitTransform(canvas, image, state.view);
    context.drawImage(
      image,
      transform.x,
      transform.y,
      image.naturalWidth * transform.scale,
      image.naturalHeight * transform.scale,
    );
    for (const region of state.regions) {
      drawPolygon(
        context,
        region.polygon_norm,
        transform,
        image,
        region.region_id === state.selectedRegionId ? "#b45309" : "#0e7490",
        false,
      );
    }
    if (state.drawing) {
      drawPolygon(context, state.drawing.points, transform, image, "#c2410c", true);
    }
  };

  const bindCanvas = (canvas: HTMLCanvasElement): void => {
    const observer = new ResizeObserver(() => draw(canvas));
    observer.observe(canvas);
    draw(canvas);
    canvas.addEventListener("contextmenu", (event) => event.preventDefault());
    const updateCrosshair = (): void => {
      const asset = selectedAsset(state);
      const visible = shouldShowCustomCrosshair(
        Boolean(asset && state.representativeIds.has(asset.asset_id)),
        Boolean(state.template),
        state.panMode,
        state.crosshair.pointerInCanvas,
        Boolean(state.image && state.imageAssetId === asset?.asset_id),
        Boolean(state.labelDraft),
      );
      const crosshair = canvas.parentElement?.querySelector<HTMLElement>("[data-drawing-crosshair]");
      if (!crosshair) return;
      crosshair.classList.toggle("visible", visible);
      crosshair.dataset.drawingCrosshair = visible ? "visible" : "hidden";
      crosshair.style.setProperty("--crosshair-x", `${Math.round(state.crosshair.x)}px`);
      crosshair.style.setProperty("--crosshair-y", `${Math.round(state.crosshair.y)}px`);
    };
    const recordMousePosition = (event: PointerEvent): void => {
      if (event.pointerType !== "mouse") return;
      const pointer = canvasPoint(event, canvas);
      state.crosshair = {
        pointerInCanvas: pointer.x >= 0 && pointer.y >= 0 && pointer.x <= canvas.clientWidth && pointer.y <= canvas.clientHeight,
        x: pointer.x,
        y: pointer.y,
      };
      updateCrosshair();
    };
    canvas.addEventListener("pointerenter", recordMousePosition);
    canvas.addEventListener("pointerleave", (event) => {
      if (event.pointerType !== "mouse") return;
      state.crosshair.pointerInCanvas = false;
      updateCrosshair();
    });
    canvas.addEventListener("pointerdown", (event) => {
      recordMousePosition(event);
      const image = state.image;
      const asset = selectedAsset(state);
      if (!image || !asset) return;
      if (event.button === 1 || event.button === 2 || state.panMode) {
        panOrigin = canvasPoint(event, canvas);
        canvas.setPointerCapture(event.pointerId);
        return;
      }
      if (event.button !== 0) return;
      if (!canAnnotate(state.representativeIds.has(asset.asset_id), Boolean(state.template))) {
        message(state.template ? "请先选择当前图片为代表图。" : "请先加载标签模板。", "error");
        render();
        return;
      }
      const point = normalizedFromPointer(canvasPoint(event, canvas), fitTransform(canvas, image, state.view), image);
      if (!point) {
        message("请在图片范围内开始圈选。", "error");
        render();
        return;
      }
      state.drawing = { points: [point] };
      canvas.setPointerCapture(event.pointerId);
      draw(canvas);
    });
    canvas.addEventListener("pointermove", (event) => {
      recordMousePosition(event);
      const image = state.image;
      if (!image) return;
      const pointer = canvasPoint(event, canvas);
      if (panOrigin) {
        state.view.panX += pointer.x - panOrigin.x;
        state.view.panY += pointer.y - panOrigin.y;
        panOrigin = pointer;
        draw(canvas);
        return;
      }
      if (!state.drawing) return;
      if (!primaryDrawingButtonIsPressed(event.buttons)) {
        state.drawing = null;
        state.labelValues = {};
        message("未完成的圈选已丢弃，请重新绘制闭合区域。", "info");
        render();
        return;
      }
      const transform = fitTransform(canvas, image, state.view);
      const point = normalizedFromPointer(pointer, transform, image);
      if (!point) return;
      const last = state.drawing.points.at(-1);
      if (!last) return;
      const screenDistance = Math.hypot(
        (point.x - last.x) * image.naturalWidth * transform.scale,
        (point.y - last.y) * image.naturalHeight * transform.scale,
      );
      const start = state.drawing.points[0];
      const startScreen = {
        x: transform.x + start.x * image.naturalWidth * transform.scale,
        y: transform.y + start.y * image.naturalHeight * transform.scale,
      };
      const distanceToStart = Math.hypot(pointer.x - startScreen.x, pointer.y - startScreen.y);
      if (canAutoClose(
        state.drawing.points,
        distanceToStart,
        Math.min(image.naturalWidth, image.naturalHeight) * transform.scale,
      )) {
        state.labelDraft = { points: state.drawing.points };
        state.drawing = null;
        state.labelValues = {};
        render();
        return;
      }
      if (screenDistance >= 2) {
        state.drawing.points.push(point);
        draw(canvas);
      }
    });
    const stopPointer = (event: PointerEvent) => {
      panOrigin = null;
      state.crosshair = clearCrosshairPointerState(state.crosshair);
      updateCrosshair();
      if (state.drawing) {
        state.drawing = null;
        state.labelValues = {};
        message("未完成的圈选已丢弃，请重新绘制闭合区域。", "info");
        render();
      }
      if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    };
    canvas.addEventListener("pointerup", stopPointer);
    canvas.addEventListener("pointercancel", stopPointer);
    canvas.addEventListener("lostpointercapture", stopPointer);
    canvas.addEventListener("wheel", (event) => {
      const image = state.image;
      if (!image) return;
      event.preventDefault();
      const pointer = canvasPoint(event, canvas);
      const before = normalizedFromPointer(pointer, fitTransform(canvas, image, state.view), image);
      if (!before) return;
      state.view.zoom = Math.min(8, Math.max(0.4, state.view.zoom * (event.deltaY < 0 ? 1.15 : 1 / 1.15)));
      const transform = fitTransform(canvas, image, state.view);
      state.view.panX += pointer.x - (transform.x + before.x * image.naturalWidth * transform.scale);
      state.view.panY += pointer.y - (transform.y + before.y * image.naturalHeight * transform.scale);
      draw(canvas);
    }, { passive: false });
  };

  const bindEvents = (): void => {
    target.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.addEventListener("click", async () => {
      try {
        const scan = await api.chooseAndScanRoot();
        if (!scan) {
          message("未选择数据根目录。", "info");
          render();
          return;
        }
        const local = await api.currentLocalDisplayScan();
        state.campuses = mergeScans(scan, local);
        const firstCampus = state.campuses[0];
        state.campusCode = firstCampus?.campusCode ?? null;
        message(`已扫描 ${state.campuses.length} 个院区。`, "success");
        await selectCase(firstCampus?.cases[0]?.case_id ?? "");
      } catch (error) {
        message(formatError(error), "error");
        render();
      }
    });
    target.querySelector<HTMLSelectElement>('[data-action="campus"]')?.addEventListener("change", async (event) => {
      state.campusCode = (event.currentTarget as HTMLSelectElement).value;
      const firstCase = selectedCampus(state)?.cases[0];
      await selectCase(firstCase?.case_id ?? "");
    });
    target.querySelectorAll<HTMLButtonElement>("[data-case-id]").forEach((button) => {
      button.addEventListener("click", () => void selectCase(button.dataset.caseId ?? ""));
    });
    target.querySelectorAll<HTMLButtonElement>('[data-action="asset"]').forEach((button) => {
      button.addEventListener("click", () => void selectAsset(Number(button.dataset.index)));
    });
    target.querySelector<HTMLButtonElement>('[data-action="previous"]')?.addEventListener("click", () => void selectAsset(state.assetIndex - 1));
    target.querySelector<HTMLButtonElement>('[data-action="next"]')?.addEventListener("click", () => void selectAsset(state.assetIndex + 1));
    target.querySelector<HTMLButtonElement>('[data-action="fit"]')?.addEventListener("click", () => {
      state.view = { zoom: 1, panX: 0, panY: 0 };
      render();
    });
    target.querySelector<HTMLButtonElement>('[data-action="pan"]')?.addEventListener("click", () => {
      state.panMode = !state.panMode;
      state.crosshair.pointerInCanvas = false;
      render();
    });
    target.querySelectorAll<HTMLButtonElement>('[data-action="representative"]').forEach((button) => {
      button.addEventListener("click", async () => {
        const currentCase = selectedCase(state);
        const assetId = button.dataset.assetId;
        if (!currentCase || !assetId) return;
        try {
          if (state.representativeIds.has(assetId)) {
            await api.deselectRepresentative(currentCase.case_id, assetId);
            state.representativeIds.delete(assetId);
            message("已取消代表图。", "success");
          } else {
            await api.selectRepresentative(currentCase.case_id, assetId);
            state.representativeIds.add(assetId);
            message("已选择为代表图。", "success");
          }
          state.crosshair.pointerInCanvas = false;
          render();
        } catch (error) {
          message(formatError(error), "error");
          render();
        }
      });
    });
    target.querySelectorAll<HTMLButtonElement>("[data-region-id]").forEach((button) => {
      button.addEventListener("click", () => {
        state.selectedRegionId = button.dataset.regionId ?? null;
        render();
      });
    });
    target.querySelector<HTMLInputElement>('[data-action="template-file"]')?.addEventListener("change", async (event) => {
      const input = event.currentTarget as HTMLInputElement;
      const file = input.files?.[0];
      if (!file) return;
      const templateId = target.querySelector<HTMLInputElement>("[data-template-id]")?.value.trim() ?? "";
      const version = Number(target.querySelector<HTMLInputElement>("[data-template-version]")?.value);
      try {
        const contents = await file.text();
        if (file.name.toLowerCase().endsWith(".txt")) {
          state.template = (await api.importLabelTxt(contents, templateId, version)).template;
        } else {
          state.template = (await api.importJsonTemplate(contents)).template;
        }
        state.templateFileName = file.name;
        message("标签模板已加载。", "success");
      } catch (error) {
        message(formatError(error), "error");
      }
      render();
    });
    const form = target.querySelector<HTMLFormElement>("[data-label-form]");
    form?.addEventListener("change", (event) => {
      const input = event.target as HTMLInputElement | HTMLSelectElement;
      const fieldId = input.dataset.labelField;
      if (!fieldId || !state.template) return;
      const field = state.template.fields.find((item) => item.id === fieldId);
      if (!field) return;
      if (field.control === "multi_select") {
        state.labelValues[fieldId] = Array.from(
          form.querySelectorAll<HTMLInputElement>(`input[data-label-field="${CSS.escape(fieldId)}"]:checked`),
          (option) => option.value,
        );
      } else {
        state.labelValues[fieldId] = input.value;
      }
      render();
    });
    form?.addEventListener("submit", async (event) => {
      event.preventDefault();
      const currentCase = selectedCase(state);
      const asset = selectedAsset(state);
      const draft = state.labelDraft;
      const template = state.template;
      if (!currentCase || !asset || !draft || !template || !labelsMeetRequiredFields(template.fields, state.labelValues)) return;
      try {
        await api.saveConfirmedRegion({
          region_id: randomRegionId(),
          asset_id: asset.asset_id,
          polygon_norm: draft.points,
          template_id: template.template_id,
          template_version: template.version,
          labels: state.labelValues,
        });
        state.labelDraft = null;
        state.labelValues = {};
        await refreshRegions();
        state.crosshair.pointerInCanvas = false;
        message("区域与标签已保存。", "success");
      } catch (error) {
        message(formatError(error), "error");
      }
      render();
    });
    target.querySelectorAll<HTMLElement>('[data-action="cancel-draft"]').forEach((element) => {
      element.addEventListener("click", (event) => {
        if (event.target === element || element.closest(".label-dialog")) {
          state.labelDraft = null;
          state.labelValues = {};
          state.drawing = null;
          state.crosshair.pointerInCanvas = false;
          message("已丢弃未保存的圈选。", "info");
          render();
        }
      });
    });
    const canvas = target.querySelector<HTMLCanvasElement>("[data-drawing-canvas]");
    if (canvas) bindCanvas(canvas);
    if (state.labelDraft) {
      target.querySelector<HTMLElement>("[data-label-dialog]")?.focus();
    }
  };

  const render = (): void => {
    target.innerHTML = mountMarkup(state);
    bindEvents();
  };

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && state.labelDraft) {
      state.labelDraft = null;
      state.labelValues = {};
      state.drawing = null;
      state.crosshair.pointerInCanvas = false;
      message("已丢弃未保存的圈选。", "info");
      render();
      return;
    }
    if (!shouldHandleMediaNavigation(event)) return;
    const currentCase = selectedCase(state);
    if (!currentCase) return;
    if (event.key === "ArrowLeft" && state.assetIndex > 0) {
      event.preventDefault();
      void selectAsset(state.assetIndex - 1);
    }
    if (event.key === "ArrowRight" && state.assetIndex < currentCase.assets.length - 1) {
      event.preventDefault();
      void selectAsset(state.assetIndex + 1);
    }
  });

  render();
}

export function boot(target = document.querySelector<HTMLDivElement>("#app")): void {
  if (!target) throw new Error("Application mount point is unavailable.");
  startApp(target);
}

if (document.querySelector("#app")) {
  boot();
}
