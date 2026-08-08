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
  template_id: string;
  template_version: number;
  template: LabelTemplate;
  labels: Record<string, LabelValue>;
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

interface CapturedFrame {
  asset_id: string;
  case_id: string;
  parent_video_asset_id: string;
  capture_time_millis: number;
  width: number;
  height: number;
  size_bytes: number;
}

interface BackupResult {
  case_id: string;
  completed_at: number;
  capture_count: number;
}

type RestoreStatus =
  | "no_session"
  | "restored"
  | "saved_root_unavailable"
  | "fixed_template_missing"
  | "fixed_template_checksum_mismatch"
  | "last_media_unavailable";

interface StartupRestore {
  status: RestoreStatus;
  scan: PublicScan | null;
  template: TemplateDescriptor | null;
  current_case_id: string | null;
  current_asset_id: string | null;
}

interface AppApi {
  chooseAndScanRoot(): Promise<PublicScan | null>;
  currentScan(): Promise<PublicScan>;
  restoreProjectSession(): Promise<StartupRestore>;
  forgetLastPosition(): Promise<void>;
  currentFixedTemplate(): Promise<TemplateDescriptor | null>;
  currentLocalDisplayScan(): Promise<LocalDisplayScan>;
  selectRepresentative(caseId: string, assetId: string): Promise<RepresentativeSelection>;
  deselectRepresentative(caseId: string, assetId: string): Promise<RepresentativeSelection>;
  representativeAssetIds(caseId: string): Promise<string[]>;
  readImageAsset(caseId: string, assetId: string): Promise<ImageAssetData>;
  savedRegionsForAsset(caseId: string, assetId: string): Promise<SavedRegion[]>;
  installFixedTomlTemplate(input: string): Promise<TemplateDescriptor>;
  replaceFixedTomlTemplate(input: string): Promise<TemplateDescriptor>;
  installFixedLabelTxt(input: string): Promise<TemplateDescriptor>;
  projectStorageLocation(): Promise<string>;
  updateRegionLabels(request: UpdateRegionLabelsRequest): Promise<void>;
  deleteRegion(caseId: string, assetId: string, regionId: string): Promise<void>;
  replaceRegion(request: ReplaceRegionRequest): Promise<void>;
  captureVideoFrame(request: CaptureFrameRequest): Promise<CapturedFrame>;
  completeCaseAndBackup(caseId: string): Promise<BackupResult>;
  saveLastPosition(caseId: string, assetId: string): Promise<void>;
  recordImageDimensions(caseId: string, assetId: string, width: number, height: number): Promise<void>;
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

interface UpdateRegionLabelsRequest {
  case_id: string;
  asset_id: string;
  region_id: string;
  template_id: string;
  template_version: number;
  labels: Record<string, LabelValue>;
}

interface ReplaceRegionRequest {
  case_id: string;
  old_region_id: string;
  replacement: SaveRegionRequest;
}

interface CaptureFrameRequest {
  case_id: string;
  video_asset_id: string;
  capture_time_millis: number;
  width: number;
  height: number;
  png_base64: string;
}

declare global {
  interface Window {
    __GWP_API__?: AppApi;
  }
}

const tauriApi: AppApi = {
  chooseAndScanRoot: () => invoke<PublicScan | null>("choose_and_scan_root"),
  currentScan: () => invoke<PublicScan>("current_scan"),
  restoreProjectSession: () => invoke<StartupRestore>("restore_project_session"),
  forgetLastPosition: () => invoke("forget_last_position"),
  currentFixedTemplate: () => invoke<TemplateDescriptor | null>("current_fixed_template"),
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
  installFixedTomlTemplate: (templateToml) =>
    invoke<TemplateDescriptor>("install_fixed_toml_template", { templateToml }),
  replaceFixedTomlTemplate: (templateToml) =>
    invoke<TemplateDescriptor>("replace_fixed_toml_template", { templateToml }),
  installFixedLabelTxt: (text) =>
    invoke<TemplateDescriptor>("install_fixed_label_txt", { text }),
  projectStorageLocation: () => invoke<string>("project_storage_location"),
  updateRegionLabels: async (request) => invoke("update_region_labels", { request }),
  deleteRegion: async (caseId, assetId, regionId) => invoke("delete_region", { caseId, assetId, regionId }),
  replaceRegion: async (request) => invoke("replace_region", { request }),
  captureVideoFrame: (request) => invoke<CapturedFrame>("capture_video_frame", { request }),
  completeCaseAndBackup: (caseId) => invoke<BackupResult>("complete_case_and_backup", { caseId }),
  saveLastPosition: async (caseId, assetId) => invoke("save_last_position", { caseId, assetId }),
  recordImageDimensions: async (caseId, assetId, width, height) =>
    invoke("record_image_dimensions", { caseId, assetId, width, height }),
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

interface EditLabelsDraft {
  regionId: string;
  template: LabelTemplate;
  values: Record<string, LabelValue>;
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
  replaceRegionId: string | null;
  editLabels: EditLabelsDraft | null;
  template: LabelTemplate | null;
  templateFileName: string | null;
  templateInstallPending: boolean;
  storageLocation: string | null;
  storageLocationVisible: boolean;
  videoTime: number;
  videoDuration: number;
  videoPlaying: boolean;
  capturePending: boolean;
  backupPending: boolean;
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
  replaceRegionId: null,
  editLabels: null,
  template: null,
  templateFileName: null,
  templateInstallPending: false,
  storageLocation: null,
  storageLocationVisible: false,
  videoTime: 0,
  videoDuration: 0,
  videoPlaying: false,
  capturePending: false,
  backupPending: false,
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

export type TemplateImportAction = "install_toml" | "replace_toml" | "install_txt" | "unsupported";

export function templateImportAction(templateLoaded: boolean, fileName: string): TemplateImportAction {
  const lowerName = fileName.toLowerCase();
  if (lowerName.endsWith(".toml")) return templateLoaded ? "replace_toml" : "install_toml";
  if (lowerName.endsWith(".txt") && !templateLoaded) return "install_txt";
  return "unsupported";
}

export const TEMPLATE_UTF8_IMPORT_ERROR =
  "标签模板编码无效或疑似损坏。请将标签模板另存为 UTF-8 后重新导入。";
export const LEGACY_TEMPLATE_ENCODING_WARNING = "当前模板疑似乱码，请用更换模板导入 UTF-8 TOML";

const TEMPLATE_ENCODING_ERROR_CODE = "INVALID_TEMPLATE_ENCODING";

// Keep this deliberately short: each is a well-known UTF-8-as-GBK artifact, not a rare Han-character ban.
const GBK_MOJIBAKE_MARKERS = ["锟斤拷", "閫夐", "瀹炵"];

const WINDOWS_1252_BYTES = new Map<number, number>([
  [0x20ac, 0x80],
  [0x201a, 0x82],
  [0x0192, 0x83],
  [0x201e, 0x84],
  [0x2026, 0x85],
  [0x2020, 0x86],
  [0x2021, 0x87],
  [0x02c6, 0x88],
  [0x2030, 0x89],
  [0x0160, 0x8a],
  [0x2039, 0x8b],
  [0x0152, 0x8c],
  [0x017d, 0x8e],
  [0x2018, 0x91],
  [0x2019, 0x92],
  [0x201c, 0x93],
  [0x201d, 0x94],
  [0x2022, 0x95],
  [0x2013, 0x96],
  [0x2014, 0x97],
  [0x02dc, 0x98],
  [0x2122, 0x99],
  [0x0161, 0x9a],
  [0x203a, 0x9b],
  [0x0153, 0x9c],
  [0x017e, 0x9e],
  [0x0178, 0x9f],
]);

function templateEncodingError(): Error & { code: string } {
  return Object.assign(new Error(TEMPLATE_UTF8_IMPORT_ERROR), {
    code: TEMPLATE_ENCODING_ERROR_CODE,
  });
}

function windows1252Byte(character: string): number | null {
  const codePoint = character.codePointAt(0);
  if (codePoint === undefined) return null;
  if (codePoint <= 0xff) return codePoint;
  return WINDOWS_1252_BYTES.get(codePoint) ?? null;
}

function byteInRange(byte: number | null | undefined, lower: number, upper: number): boolean {
  return byte !== null && byte !== undefined && byte >= lower && byte <= upper;
}

function hasObviousMojibake(value: string): boolean {
  if (GBK_MOJIBAKE_MARKERS.some((marker) => value.includes(marker))) return true;

  const bytes = Array.from(value, windows1252Byte);
  for (let index = 0; index < bytes.length; index += 1) {
    const lead = bytes[index];
    if (lead === null || lead === undefined) continue;

    const second = bytes[index + 1];
    const third = bytes[index + 2];
    const fourth = bytes[index + 3];
    if (
      ((lead >= 0xc2 && lead <= 0xdf) && byteInRange(second, 0x80, 0xbf)) ||
      (lead === 0xe0 && byteInRange(second, 0xa0, 0xbf) && byteInRange(third, 0x80, 0xbf)) ||
      (((lead >= 0xe1 && lead <= 0xec) || (lead >= 0xee && lead <= 0xef)) &&
        byteInRange(second, 0x80, 0xbf) &&
        byteInRange(third, 0x80, 0xbf)) ||
      (lead === 0xed && byteInRange(second, 0x80, 0x9f) && byteInRange(third, 0x80, 0xbf)) ||
      (lead === 0xf0 &&
        byteInRange(second, 0x90, 0xbf) &&
        byteInRange(third, 0x80, 0xbf) &&
        byteInRange(fourth, 0x80, 0xbf)) ||
      ((lead >= 0xf1 && lead <= 0xf3) &&
        byteInRange(second, 0x80, 0xbf) &&
        byteInRange(third, 0x80, 0xbf) &&
        byteInRange(fourth, 0x80, 0xbf)) ||
      (lead === 0xf4 &&
        byteInRange(second, 0x80, 0x8f) &&
        byteInRange(third, 0x80, 0xbf) &&
        byteInRange(fourth, 0x80, 0xbf))
    ) {
      return true;
    }
  }
  return false;
}

function hasPrivateUseCodePoint(value: string): boolean {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (
      codePoint !== undefined &&
      ((codePoint >= 0xe000 && codePoint <= 0xf8ff) ||
        (codePoint >= 0xf0000 && codePoint <= 0xffffd) ||
        (codePoint >= 0x100000 && codePoint <= 0x10fffd))
    ) {
      return true;
    }
  }
  return false;
}

export function validateTemplateTextEncoding(value: string): void {
  // These byte-like sequences are characteristic of UTF-8 rendered as Latin-1/Windows-1252.
  if (templateTextHasEncodingDamage(value)) {
    throw templateEncodingError();
  }
}

function templateTextHasEncodingDamage(value: string): boolean {
  return value.includes("\uFFFD") || hasPrivateUseCodePoint(value) || hasObviousMojibake(value);
}

export function templateHasEncodingDamage(template: LabelTemplate | null | undefined): boolean {
  return Boolean(
    template?.fields.some(
      (field) =>
        templateTextHasEncodingDamage(field.label) ||
        field.options.some((option) => templateTextHasEncodingDamage(option.label)),
    ),
  );
}

export function templateAllowsNewAnnotations(template: LabelTemplate | null | undefined): boolean {
  return Boolean(template && !templateHasEncodingDamage(template));
}

export function decodeUtf8Template(bytes: ArrayBuffer | Uint8Array): string {
  let value: string;
  try {
    value = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw templateEncodingError();
  }
  validateTemplateTextEncoding(value);
  return value;
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
    [TEMPLATE_ENCODING_ERROR_CODE]: TEMPLATE_UTF8_IMPORT_ERROR,
    INVALID_CAMPUS_LAYOUT: "数据根目录至少需要包含一个有效院区目录。",
    SOURCE_WORK_OVERLAP: "数据根目录与工作目录重叠，已拒绝操作。",
    REPRESENTATIVE_HAS_REGIONS: "该代表图已有保存区域，不能取消选择。",
    NOT_IMAGE_ASSET: "只有图片可以作为代表图。",
    ASSET_NOT_REPRESENTATIVE: "请先选择当前图片为代表图。",
    TEMPLATE_NOT_LOADED: "请先加载标签模板。",
    INVALID_TEMPLATE: "标签模板内容无效或编码损坏。请将标签模板另存为 UTF-8 后重新导入。",
    FIXED_TEMPLATE_MISSING: "固定标签模板不可用，请重新选择 label-template-v1.toml。",
    FIXED_TEMPLATE_CHECKSUM_MISMATCH: "固定标签模板校验失败，请联系项目管理员后重新导入。",
    FIXED_TEMPLATE_ALREADY_INSTALLED: "当前模板未能更换，请确认选择的是新的 TOML 模板。",
    TEMPLATE_REPLACEMENT_IDENTITY_CONFLICT: "该模板 ID 和 version 已用于标注。请在 TOML 中增加 version，或更换 template_id。",
    TEMPLATE_NOT_ACTIVE: "当前标注仍引用旧模板，请刷新后使用当前模板重试。",
    TEMPLATE_SNAPSHOT_CONFLICT: "该模板版本已经用于标注。请在 TOML 中增加 version，或更换 template_id 后再导入。",
    WORK_ROOT_UNAVAILABLE: "无法使用软件安装目录旁的数据保存位置。请打开“数据保存位置”核对目录、当前用户权限和磁盘空间；软件不会回退到 C 盘或 %LOCALAPPDATA%。",
    UNSAFE_DERIVED_PATH: "数据保存路径包含不安全的链接、联接点或非普通文件，已拒绝操作。请检查“数据保存位置”后重试。",
    SAVED_ROOT_UNAVAILABLE: "上次的数据目录不可用，请连接磁盘后重新选择。",
    NOT_VIDEO_ASSET: "当前媒体不是 MP4 视频。",
    INVALID_CAPTURE: "无法截取当前视频帧。请确认本机可播放该 MP4。",
    REGION_NOT_FOUND: "该区域已不存在，请刷新后重试。",
    REGION_NOT_ACTIVE: "该区域已被删除或替换，请刷新后重试。",
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

export function formatVideoTime(value: number): string {
  const seconds = Number.isFinite(value) && value >= 0 ? Math.floor(value) : 0;
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

function videoFrameToBase64(video: HTMLVideoElement): { pngBase64: string; width: number; height: number } | null {
  const width = video.videoWidth;
  const height = video.videoHeight;
  if (width < 1 || height < 1) return null;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) return null;
  context.drawImage(video, 0, 0, width, height);
  const dataUrl = canvas.toDataURL("image/png");
  const prefix = "data:image/png;base64,";
  return dataUrl.startsWith(prefix) ? { pngBase64: dataUrl.slice(prefix.length), width, height } : null;
}

const viewportStorageKey = "gwp-annotator-v02-viewport";

function loadSavedViewport(caseId: string, assetId: string): ViewState | null {
  try {
    const value = JSON.parse(localStorage.getItem(viewportStorageKey) ?? "null") as Partial<ViewState> & { caseId?: string; assetId?: string } | null;
    if (!value || value.caseId !== caseId || value.assetId !== assetId) return null;
    if (![value.zoom, value.panX, value.panY].every(Number.isFinite)) return null;
    return { zoom: Math.min(8, Math.max(0.4, value.zoom as number)), panX: value.panX as number, panY: value.panY as number };
  } catch {
    return null;
  }
}

function saveViewport(caseId: string, assetId: string, view: ViewState): void {
  try {
    localStorage.setItem(viewportStorageKey, JSON.stringify({ caseId, assetId, ...view }));
  } catch {
    // Viewport recovery is optional and must not block a saved annotation.
  }
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
  const templateReady = templateAllowsNewAnnotations(state.template);
  const annotationReady = canAnnotate(isRepresentative, templateReady);
  const imageReady = Boolean(state.image && state.imageAssetId === asset?.asset_id);
  const crosshairVisible = shouldShowCustomCrosshair(
    isRepresentative,
    templateReady,
    state.panMode,
    state.crosshair.pointerInCanvas,
    imageReady,
    Boolean(state.labelDraft),
  );
  const crosshairReady = annotationReady && !state.panMode && imageReady && !state.labelDraft;
  const templateInputDisabled = state.templateInstallPending || Boolean(state.labelDraft) || Boolean(state.editLabels);
  const media = currentCase?.assets ?? [];
  const currentIndex = asset ? state.assetIndex + 1 : 0;

  return `
    <main class="workbench" aria-labelledby="app-title">
      <header class="app-header">
        <div><h1 id="app-title">GWP Annotator</h1><p>本地医生标注工作台</p></div>
        <div class="header-actions">
          <button type="button" data-action="choose-root">更换数据目录</button>
          <button type="button" data-action="forget-position">忘记上次位置</button>
          <button type="button" data-action="storage-location" aria-expanded="${state.storageLocationVisible}">数据保存位置</button>
          <span class="offline-status" data-testid="offline-status">本地离线</span>
        </div>
      </header>
      <p class="notice ${state.messageKind}" role="status" aria-live="polite">${escapeHtml(state.message)}</p>
      ${state.storageLocationVisible ? `<section class="storage-location" aria-label="数据保存位置"><strong>数据保存位置</strong><output>${escapeHtml(state.storageLocation ?? "正在读取…")}</output><span>Windows 中该目录位于软件安装目录旁边；安装在 D 盘时，标注数据也保存在 D 盘。</span></section>` : ""}
      <section class="template-strip" aria-label="标签模板">
        <strong>标签模板</strong>
        <span>${state.template ? `当前模板：${escapeHtml(state.template.template_id)} v${state.template.version}` : "未加载。请选择 label-template-v1.toml。"}</span>
        <label class="file-button ${templateInputDisabled ? "disabled" : ""}">${state.templateInstallPending ? "正在验证…" : state.template ? "更换模板" : "导入模板"}<input data-action="template-file" type="file" accept="${state.template ? ".toml,text/plain" : ".toml,.txt,text/plain"}" aria-label="${state.template ? "更换标签模板" : "导入标签模板"}" ${templateInputDisabled ? "disabled" : ""}></label>
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
              <button type="button" data-action="complete-backup" ${currentCase && !state.backupPending ? "" : "disabled"}>${state.backupPending ? "正在备份…" : "完成本病例并备份"}</button>
            </div>
          </div>
          <div class="canvas-frame ${state.panMode ? "pan-mode" : ""} ${crosshairReady ? "crosshair-ready" : ""}" data-canvas-frame>
            ${!asset ? `<p class="empty-copy">从左侧选择病例。</p>` : ""}
            ${asset?.media_kind === "video" && currentCase ? `<div class="video-player"><video data-video-player preload="metadata" src="gwp-media://localhost/video/${encodeURIComponent(currentCase.case_id)}/${encodeURIComponent(asset.asset_id)}"></video><div class="video-controls"><button type="button" data-action="video-play">${state.videoPlaying ? "暂停" : "播放"}</button><input data-action="video-seek" type="range" min="0" max="${Math.max(state.videoDuration, 0)}" value="${Math.min(state.videoTime, state.videoDuration || 0)}" step="0.01" aria-label="视频进度"><output data-video-time>${formatVideoTime(state.videoTime)} / ${formatVideoTime(state.videoDuration)}</output><button type="button" data-action="capture-frame" ${state.capturePending ? "disabled" : ""}>${state.capturePending ? "正在截帧…" : "截取当前帧"}</button></div><p>截帧仅保存到本应用工作目录，并归属于当前病例；原始病例目录保持只读。</p></div>` : ""}
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
                  : `<span class="deferred-badge">可播放与截帧</span>`;
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
                  `<article class="region-item ${region.region_id === state.selectedRegionId ? "selected" : ""}"><button type="button" data-region-id="${escapeHtml(region.region_id)}">区域 ${String(index + 1).padStart(2, "0")}${state.template && (region.template_id !== state.template.template_id || region.template_version !== state.template.version) ? `<small>旧模板 v${region.template_version}</small>` : ""}</button>${region.region_id === state.selectedRegionId ? `<div class="region-actions"><button type="button" data-action="edit-region" data-region-id="${escapeHtml(region.region_id)}">修改标签</button><button type="button" data-action="replace-region" data-region-id="${escapeHtml(region.region_id)}" ${templateReady ? "" : "disabled"}>删除后重画</button><button type="button" data-action="delete-region" data-region-id="${escapeHtml(region.region_id)}">删除</button></div>` : ""}</article>`,
              )
              .join("") || `<p class="empty-copy">尚未保存区域。</p>`}
          </div>
          <div class="annotation-state ${annotationReady ? "ready" : ""}">${isImage ? annotationReady ? "可圈选" : isRepresentative ? state.template && !templateReady ? "请更换模板" : "等待模板" : "等待代表图" : "图片标注不可用"}</div>
        </aside>
      </div>
      ${state.labelDraft && state.template ? `
        <div class="modal-backdrop" data-action="cancel-draft">
          <section class="label-dialog" role="dialog" aria-modal="true" aria-labelledby="label-dialog-title" data-label-dialog tabindex="-1">
            <header><h2 id="label-dialog-title">填写区域标签</h2><button type="button" data-action="cancel-draft" aria-label="取消">取消</button></header>
            <p>${state.replaceRegionId ? "将替换已保存区域；确认后旧区域保留操作记录但不再用于训练。" : "区域尚未保存。完成必填项后才可确认。"}</p>
            <form data-label-form>
              ${state.template.fields.map((field) => controlMarkup(field, state.labelValues[field.id])).join("")}
              <div class="dialog-actions"><button type="button" data-action="cancel-draft">取消</button><button type="submit" ${labelsMeetRequiredFields(state.template.fields, state.labelValues) ? "" : "disabled"}>OK</button></div>
            </form>
          </section>
        </div>` : ""}
      ${state.editLabels ? `<div class="modal-backdrop" data-action="cancel-edit"><section class="label-dialog" role="dialog" aria-modal="true" aria-labelledby="edit-label-title" tabindex="-1" data-edit-dialog><header><h2 id="edit-label-title">修改区域标签</h2><button type="button" data-action="cancel-edit">取消</button></header><p>使用该区域原来的 ${escapeHtml(state.editLabels.template.template_id)} v${state.editLabels.template.version} 模板；确认后保留修正记录。</p><form data-edit-label-form>${state.editLabels.template.fields.map((field) => controlMarkup(field, state.editLabels?.values[field.id])).join("")}<div class="dialog-actions"><button type="button" data-action="cancel-edit">取消</button><button type="submit" ${labelsMeetRequiredFields(state.editLabels.template.fields, state.editLabels.values) ? "" : "disabled"}>确认修改</button></div></form></section></div>` : ""}
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

  const reconcileFailedTemplateReplacement = async (replacementError: unknown, fileName: string): Promise<void> => {
    try {
      const activeTemplate = await api.currentFixedTemplate();
      state.template = activeTemplate?.template ?? null;
      state.templateFileName = activeTemplate ? fileName : null;
      state.labelValues = {};
      message(
        `更换标签模板失败：${formatError(replacementError)}；${activeTemplate ? "已根据后端当前模板刷新界面。" : "后端当前未加载标签模板。"}`,
        "error",
      );
    } catch {
      message(`更换标签模板失败：${formatError(replacementError)}；未能读取后端当前模板，界面保留原模板。`, "error");
    }
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
      void api.recordImageDimensions(requestedCase, requestedAsset, image.naturalWidth, image.naturalHeight).catch(() => {
        if (selectedCase(state)?.case_id === requestedCase && selectedAsset(state)?.asset_id === requestedAsset) {
          message("图片尺寸未能记录；不影响当前标注。", "info");
          render();
        }
      });
    } catch (error) {
      if (selectedAsset(state)?.asset_id === requestedAsset) {
        state.imageLoading = false;
        message(formatError(error), "error");
      }
    }
    render();
  };

  const selectCase = async (caseId: string): Promise<void> => {
    const previousCase = selectedCase(state);
    const previousAsset = selectedAsset(state);
    if (previousCase && previousAsset) saveViewport(previousCase.case_id, previousAsset.asset_id, state.view);
    const campus = selectedCampus(state);
    const nextCase = campus?.cases.find((item) => item.case_id === caseId);
    if (!nextCase) return;
    state.caseId = nextCase.case_id;
    state.assetIndex = 0;
    state.representativeIds = new Set(await api.representativeAssetIds(nextCase.case_id));
    state.regions = [];
    state.selectedRegionId = null;
    state.view = loadSavedViewport(nextCase.case_id, nextCase.assets[0]?.asset_id ?? "") ?? { zoom: 1, panX: 0, panY: 0 };
    state.drawing = null;
    state.labelDraft = null;
    state.editLabels = null;
    state.replaceRegionId = null;
    state.crosshair.pointerInCanvas = false;
    await refreshRegions();
    await loadImage();
    const asset = selectedAsset(state);
    if (asset) await api.saveLastPosition(nextCase.case_id, asset.asset_id);
  };

  const selectAsset = async (index: number): Promise<void> => {
    const currentCase = selectedCase(state);
    if (!currentCase || index < 0 || index >= currentCase.assets.length) return;
    const previousAsset = selectedAsset(state);
    if (previousAsset) saveViewport(currentCase.case_id, previousAsset.asset_id, state.view);
    state.assetIndex = index;
    state.view = loadSavedViewport(currentCase.case_id, currentCase.assets[index].asset_id) ?? { zoom: 1, panX: 0, panY: 0 };
    state.drawing = null;
    state.labelDraft = null;
    state.editLabels = null;
    state.replaceRegionId = null;
    state.crosshair.pointerInCanvas = false;
    await refreshRegions();
    await loadImage();
    await api.saveLastPosition(currentCase.case_id, currentCase.assets[index].asset_id);
  };

  const applyScan = async (scan: PublicScan, preferredCaseId?: string | null, preferredAssetId?: string | null): Promise<void> => {
    const local = await api.currentLocalDisplayScan();
    state.campuses = mergeScans(scan, local);
    const preferredCampus = state.campuses.find((campus) => campus.cases.some((item) => item.case_id === preferredCaseId));
    const campus = preferredCampus ?? state.campuses[0];
    state.campusCode = campus?.campusCode ?? null;
    const activeCase = campus?.cases.find((item) => item.case_id === preferredCaseId) ?? campus?.cases[0];
    if (!activeCase) {
      render();
      return;
    }
    state.caseId = activeCase.case_id;
    state.assetIndex = Math.max(0, activeCase.assets.findIndex((asset) => asset.asset_id === preferredAssetId));
    const restoredAsset = activeCase.assets[state.assetIndex];
    state.view = restoredAsset ? loadSavedViewport(activeCase.case_id, restoredAsset.asset_id) ?? { zoom: 1, panX: 0, panY: 0 } : { zoom: 1, panX: 0, panY: 0 };
    state.representativeIds = new Set(await api.representativeAssetIds(activeCase.case_id));
    state.regions = [];
    state.selectedRegionId = null;
    await refreshRegions();
    await loadImage();
    const activeAsset = selectedAsset(state);
    if (activeAsset) await api.saveLastPosition(activeCase.case_id, activeAsset.asset_id);
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
        templateAllowsNewAnnotations(state.template),
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
      const templateReady = templateAllowsNewAnnotations(state.template);
      if (!canAnnotate(state.representativeIds.has(asset.asset_id), templateReady)) {
        message(
          state.template && !templateReady
            ? LEGACY_TEMPLATE_ENCODING_WARNING
            : state.template
              ? "请先选择当前图片为代表图。"
              : "请先加载标签模板。",
          "error",
        );
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
        await applyScan(scan);
        try {
          const fixedTemplate = await api.currentFixedTemplate();
          state.template = fixedTemplate?.template ?? null;
          state.templateFileName = fixedTemplate ? "label-template-v1.toml" : null;
        } catch (error) {
          state.template = null;
          state.templateFileName = null;
          throw error;
        }
        message(
          templateHasEncodingDamage(state.template)
            ? LEGACY_TEMPLATE_ENCODING_WARNING
            : `已扫描 ${state.campuses.length} 个院区。`,
          templateHasEncodingDamage(state.template) ? "error" : "success",
        );
        render();
      } catch (error) {
        message(formatError(error), "error");
        render();
      }
    });
    target.querySelector<HTMLButtonElement>('[data-action="forget-position"]')?.addEventListener("click", async () => {
      if (!window.confirm("忘记上次工作位置？下次启动将不再自动打开当前数据目录。")) return;
      try {
        await api.forgetLastPosition();
        message("已忘记上次工作位置。当前已打开的数据不会关闭。", "success");
      } catch (error) {
        message(formatError(error), "error");
      }
      render();
    });
    target.querySelector<HTMLButtonElement>('[data-action="storage-location"]')?.addEventListener("click", async () => {
      state.storageLocationVisible = !state.storageLocationVisible;
      render();
      if (!state.storageLocationVisible || state.storageLocation) return;
      try {
        state.storageLocation = await api.projectStorageLocation();
      } catch (error) {
        state.storageLocationVisible = false;
        message(formatError(error), "error");
      }
      render();
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
    target.querySelector<HTMLButtonElement>('[data-action="complete-backup"]')?.addEventListener("click", async () => {
      const currentCase = selectedCase(state);
      if (!currentCase || !window.confirm("确认完成本病例并创建可恢复备份？")) return;
      state.backupPending = true;
      render();
      try {
        const result = await api.completeCaseAndBackup(currentCase.case_id);
        message(`病例已完成并备份（包含 ${result.capture_count} 张截帧）。`, "success");
      } catch (error) {
        message(formatError(error), "error");
      }
      state.backupPending = false;
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
    target.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.addEventListener("click", () => {
      const regionId = target.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.dataset.regionId;
      const region = state.regions.find((item) => item.region_id === regionId);
      if (!region) return;
      state.editLabels = {
        regionId: region.region_id,
        template: region.template,
        values: structuredClone(region.labels),
      };
      render();
    });
    target.querySelector<HTMLButtonElement>('[data-action="delete-region"]')?.addEventListener("click", async (event) => {
      const regionId = (event.currentTarget as HTMLButtonElement).dataset.regionId;
      const currentCase = selectedCase(state);
      const asset = selectedAsset(state);
      if (!regionId || !currentCase || !asset || !window.confirm("确认删除此区域？删除后不会用于训练，但会保留操作记录。")) return;
      try {
        await api.deleteRegion(currentCase.case_id, asset.asset_id, regionId);
        await refreshRegions();
        message("区域已删除。", "success");
      } catch (error) {
        message(formatError(error), "error");
      }
      render();
    });
    target.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.addEventListener("click", (event) => {
      if (!templateAllowsNewAnnotations(state.template)) {
        message(state.template ? LEGACY_TEMPLATE_ENCODING_WARNING : "请先加载标签模板。", "error");
        render();
        return;
      }
      const regionId = (event.currentTarget as HTMLButtonElement).dataset.regionId;
      if (!regionId || !window.confirm("重新圈选后将替换此区域。原区域会保留操作记录。继续吗？")) return;
      state.replaceRegionId = regionId;
      state.selectedRegionId = regionId;
      message("请重新圈选该区域；保存后将替换旧区域。", "info");
      render();
    });
    target.querySelector<HTMLInputElement>('[data-action="template-file"]')?.addEventListener("change", async (event) => {
      const input = event.currentTarget as HTMLInputElement;
      const file = input.files?.[0];
      if (!file) return;
      const replacing = Boolean(state.template);
      const action = templateImportAction(replacing, file.name);
      if (action === "unsupported") {
        message("正式模板支持 .toml；.txt 仅用于首次兼容导入。", "error");
        render();
        return;
      }
      if (
        replacing
        && !window.confirm(
          "确认更换当前标签模板？已有区域继续保留并使用原模板；新圈选和删除后重画将使用新模板，不会自动转换旧标签。",
        )
      ) {
        input.value = "";
        return;
      }
      state.templateInstallPending = true;
      render();
      try {
        const contents = decodeUtf8Template(await file.arrayBuffer());
        if (action === "install_txt") {
          state.template = (await api.installFixedLabelTxt(contents)).template;
        } else if (action === "replace_toml") {
          try {
            state.template = (await api.replaceFixedTomlTemplate(contents)).template;
          } catch (error) {
            await reconcileFailedTemplateReplacement(error, file.name);
            return;
          }
        } else {
          state.template = (await api.installFixedTomlTemplate(contents)).template;
        }
        state.templateFileName = file.name;
        state.labelValues = {};
        message(
          replacing
            ? "标签模板已更换。旧区域保留原模板，新圈选使用当前模板。"
            : "标签模板已安装，后续启动将自动加载。",
          "success",
        );
      } catch (error) {
        message(formatError(error), "error");
      } finally {
        state.templateInstallPending = false;
        render();
      }
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
      if (
        !currentCase ||
        !asset ||
        !draft ||
        !template ||
        !templateAllowsNewAnnotations(template) ||
        !labelsMeetRequiredFields(template.fields, state.labelValues)
      ) return;
      try {
        const request: SaveRegionRequest = {
          region_id: randomRegionId(),
          asset_id: asset.asset_id,
          polygon_norm: draft.points,
          template_id: template.template_id,
          template_version: template.version,
          labels: state.labelValues,
        };
        if (state.replaceRegionId) {
          await api.replaceRegion({ case_id: currentCase.case_id, old_region_id: state.replaceRegionId, replacement: request });
        } else {
          await api.saveConfirmedRegion(request);
        }
        state.labelDraft = null;
        state.labelValues = {};
        state.replaceRegionId = null;
        await refreshRegions();
        state.crosshair.pointerInCanvas = false;
        message("区域与标签已保存。", "success");
      } catch (error) {
        message(formatError(error), "error");
      }
      render();
    });
    const editForm = target.querySelector<HTMLFormElement>("[data-edit-label-form]");
    editForm?.addEventListener("change", (event) => {
      const input = event.target as HTMLInputElement | HTMLSelectElement;
      const fieldId = input.dataset.labelField;
      const edit = state.editLabels;
      const template = edit?.template;
      if (!fieldId || !edit || !template) return;
      const field = template.fields.find((item) => item.id === fieldId);
      if (!field) return;
      edit.values[fieldId] = field.control === "multi_select"
        ? Array.from(editForm.querySelectorAll<HTMLInputElement>(`input[data-label-field="${CSS.escape(fieldId)}"]:checked`), (option) => option.value)
        : input.value;
      render();
    });
    editForm?.addEventListener("submit", async (event) => {
      event.preventDefault();
      const currentCase = selectedCase(state);
      const asset = selectedAsset(state);
      const edit = state.editLabels;
      const template = edit?.template;
      if (!currentCase || !asset || !edit || !template || !labelsMeetRequiredFields(template.fields, edit.values)) return;
      try {
        await api.updateRegionLabels({ case_id: currentCase.case_id, asset_id: asset.asset_id, region_id: edit.regionId, template_id: template.template_id, template_version: template.version, labels: edit.values });
        state.editLabels = null;
        await refreshRegions();
        message("区域标签已修改。", "success");
      } catch (error) {
        message(formatError(error), "error");
      }
      render();
    });
    target.querySelectorAll<HTMLElement>('[data-action="cancel-edit"]').forEach((element) => {
      element.addEventListener("click", () => {
        state.editLabels = null;
        render();
      });
    });
    target.querySelectorAll<HTMLElement>('[data-action="cancel-draft"]').forEach((element) => {
      element.addEventListener("click", (event) => {
        if (event.target === element || element.closest(".label-dialog")) {
          state.labelDraft = null;
          state.labelValues = {};
          state.drawing = null;
          state.replaceRegionId = null;
          state.crosshair.pointerInCanvas = false;
          message("已丢弃未保存的圈选。", "info");
          render();
        }
      });
    });
    const video = target.querySelector<HTMLVideoElement>("[data-video-player]");
    if (video) {
      const syncVideoState = () => {
        state.videoTime = video.currentTime;
        state.videoDuration = Number.isFinite(video.duration) ? video.duration : 0;
        state.videoPlaying = !video.paused && !video.ended;
        const output = target.querySelector<HTMLOutputElement>("[data-video-time]");
        const seek = target.querySelector<HTMLInputElement>('[data-action="video-seek"]');
        if (output) output.value = `${formatVideoTime(state.videoTime)} / ${formatVideoTime(state.videoDuration)}`;
        if (seek) {
          seek.max = String(state.videoDuration || 0);
          seek.value = String(Math.min(state.videoTime, state.videoDuration || 0));
        }
      };
      video.addEventListener("loadedmetadata", syncVideoState);
      video.addEventListener("timeupdate", syncVideoState);
      video.addEventListener("play", syncVideoState);
      video.addEventListener("pause", syncVideoState);
      video.addEventListener("error", () => {
        message("本机无法播放此 MP4 编码，无法截帧。请使用本机支持的编码。", "error");
        render();
      });
      target.querySelector<HTMLButtonElement>('[data-action="video-play"]')?.addEventListener("click", async (event) => {
        try {
          if (video.paused) await video.play();
          else video.pause();
          syncVideoState();
          (event.currentTarget as HTMLButtonElement).textContent = state.videoPlaying ? "暂停" : "播放";
        } catch {
          message("本机无法播放此 MP4 编码。", "error");
          render();
        }
      });
      target.querySelector<HTMLInputElement>('[data-action="video-seek"]')?.addEventListener("input", (event) => {
        video.currentTime = Number((event.currentTarget as HTMLInputElement).value);
        syncVideoState();
      });
      target.querySelector<HTMLButtonElement>('[data-action="capture-frame"]')?.addEventListener("click", async () => {
        const currentCase = selectedCase(state);
        const asset = selectedAsset(state);
        if (!currentCase || !asset || asset.media_kind !== "video") return;
        const frame = videoFrameToBase64(video);
        if (!frame) {
          message("当前视频帧尚不可用，播放或拖动到有效位置后重试。", "error");
          render();
          return;
        }
        state.capturePending = true;
        render();
        try {
          const captured = await api.captureVideoFrame({
            case_id: currentCase.case_id,
            video_asset_id: asset.asset_id,
            capture_time_millis: Math.round(video.currentTime * 1000),
            width: frame.width,
            height: frame.height,
            png_base64: frame.pngBase64,
          });
          const scan = await api.currentLocalDisplayScan();
          const publicScan = await api.currentScan();
          state.campuses = mergeScans(publicScan, scan);
          const caseAfterCapture = selectedCase(state);
          const index = caseAfterCapture?.assets.findIndex((item) => item.asset_id === captured.asset_id) ?? -1;
          message("截帧已保存到本应用工作目录，并加入当前病例。", "success");
          if (index >= 0) await selectAsset(index);
        } catch (error) {
          message(formatError(error), "error");
        }
        state.capturePending = false;
        render();
      });
    }
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
      state.replaceRegionId = null;
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
  void (async () => {
    try {
      const restored = await api.restoreProjectSession();
      if (restored.scan) {
        state.template = restored.template?.template ?? null;
        await applyScan(restored.scan, restored.current_case_id, restored.current_asset_id);
      }
      const restoreMessages: Record<RestoreStatus, { text: string; kind: State["messageKind"] }> = {
        no_session: { text: "请选择本地数据根目录。", kind: "info" },
        restored: { text: "已恢复上次的数据目录、模板与浏览位置。", kind: "success" },
        saved_root_unavailable: { text: "上次的数据目录不可用，请连接磁盘后重新选择。", kind: "error" },
        fixed_template_missing: { text: "已恢复数据目录，但当前模板不可用；请重新导入对应的 TOML 模板。", kind: "error" },
        fixed_template_checksum_mismatch: { text: "当前模板校验失败；请联系项目管理员后重新导入。", kind: "error" },
        last_media_unavailable: { text: "已恢复数据目录和模板，但上次媒体不存在，已打开该病例的首个媒体。", kind: "info" },
      };
      const restoreMessage = restoreMessages[restored.status];
      message(
        templateHasEncodingDamage(state.template)
          ? LEGACY_TEMPLATE_ENCODING_WARNING
          : restoreMessage.text,
        templateHasEncodingDamage(state.template) ? "error" : restoreMessage.kind,
      );
    } catch (error) {
      message(formatError(error), "error");
    }
    render();
  })();
}

export function boot(target = document.querySelector<HTMLDivElement>("#app")): void {
  if (!target) throw new Error("Application mount point is unavailable.");
  startApp(target);
}

if (document.querySelector("#app")) {
  boot();
}
