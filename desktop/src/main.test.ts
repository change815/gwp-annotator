import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  autoCloseThreshold,
  boot,
  brushDiameterCssPixels,
  canAnnotate,
  canAutoClose,
  canStartRefinement,
  clearCrosshairPointerState,
  customCrosshairMarkup,
  decodeUtf8Template,
  formatVideoTime,
  labelsMeetRequiredFields,
  LEGACY_TEMPLATE_ENCODING_WARNING,
  polygonArea,
  primaryDrawingButtonIsPressed,
  refinementFailureMessage,
  shouldHandleMediaNavigation,
  shouldShowCustomCrosshair,
  TEMPLATE_UTF8_IMPORT_ERROR,
  templateHasEncodingDamage,
  templateAllowsNewAnnotations,
  templateImportAction,
  type LabelTemplate,
  type PointNorm,
  type TemplateField,
} from "./main";
import { refinePolygonWithStroke } from "./refinement";

const triangle: PointNorm[] = [
  { x: 0.1, y: 0.1 },
  { x: 0.8, y: 0.1 },
  { x: 0.4, y: 0.8 },
];

const templateV1: LabelTemplate = {
  format: "gwa-label-template",
  version: 1,
  template_id: "template-test",
  fields: [],
};

const templateV2: LabelTemplate = { ...templateV1, version: 2 };

const officialSerosaTemplate: LabelTemplate = {
  format: "gwa-label-template",
  version: 1,
  template_id: "gastric-serosa-white-plaque-v1",
  fields: [
    {
      id: "serosal_classification",
      label: "浆膜分型",
      control: "single_select",
      required: true,
      options: [
        { id: "normal", label: "正常型" },
        { id: "reactive", label: "反应型" },
        { id: "protruding_nodular", label: "突出结节型" },
        { id: "flat_nodular", label: "扁平结节型" },
        { id: "tendon_like", label: "腱状型" },
        { id: "multicolored_diffuse", label: "多彩弥漫型" },
      ],
    },
    {
      id: "serosal_white_plaque_classification",
      label: "浆膜白斑分型",
      control: "single_select",
      required: true,
      options: [
        { id: "protruding", label: "突出型" },
        { id: "flat", label: "扁平型" },
        { id: "depressed", label: "凹陷型" },
      ],
    },
    {
      id: "serosal_white_plaque_confidence",
      label: "浆膜白斑置信度",
      control: "single_select",
      required: true,
      options: [
        { id: "confidence_50", label: "50%" },
        { id: "confidence_80", label: "80%" },
        { id: "confidence_100", label: "100%" },
      ],
    },
  ],
};

const editableTemplate: LabelTemplate = {
  ...templateV1,
  template_id: "template-editable",
  fields: [
    {
      id: "field_editable",
      label: "Synthetic choice",
      control: "single_select",
      required: true,
      options: [
        { id: "option_alpha", label: "Option alpha" },
        { id: "option_beta", label: "Option beta" },
      ],
    },
  ],
};

const syntheticRegion = {
  region_id: "REGION-SYNTHETIC",
  asset_id: "ASSET-SYNTHETIC",
  polygon_norm: [
    { x: 0.2, y: 0.2 },
    { x: 0.8, y: 0.2 },
    { x: 0.8, y: 0.8 },
    { x: 0.2, y: 0.8 },
  ],
  template_id: templateV1.template_id,
  template_version: templateV1.version,
  template: templateV1,
  labels: {},
};

const editableSyntheticRegion = {
  ...syntheticRegion,
  template_id: editableTemplate.template_id,
  template_version: editableTemplate.version,
  template: editableTemplate,
  labels: { field_editable: "option_alpha" },
};

const officialSerosaRegion = {
  ...syntheticRegion,
  template_id: officialSerosaTemplate.template_id,
  template_version: officialSerosaTemplate.version,
  template: officialSerosaTemplate,
  labels: {
    serosal_classification: "normal",
    serosal_white_plaque_classification: "flat",
    serosal_white_plaque_confidence: "confidence_80",
  },
};

const legacyDamagedTemplate: LabelTemplate = {
  ...templateV1,
  fields: [
    {
      id: "field_legacy",
      label: "閫夐」",
      control: "dropdown",
      required: true,
      options: [{ id: "option_legacy", label: "锟斤拷" }],
    },
  ],
};

function templateDescriptor(template: LabelTemplate, sha256: string): Record<string, unknown> {
  return {
    template_id: template.template_id,
    version: template.version,
    sha256,
    template,
  };
}

function restoredWithTemplate(template: LabelTemplate, sha256: string): Record<string, unknown> {
  return {
    status: "restored",
    scan: { campuses: [] },
    template: templateDescriptor(template, sha256),
    current_case_id: null,
    current_asset_id: null,
  };
}

function mockAppApi(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    restoreProjectSession: vi.fn().mockResolvedValue({
      status: "no_session",
      scan: null,
      template: null,
      current_case_id: null,
      current_asset_id: null,
    }),
    currentLocalDisplayScan: vi.fn().mockResolvedValue({ campuses: [] }),
    projectStorageLocation: vi.fn().mockResolvedValue("D:\\Apps\\GWP Annotator Data\\PROJECT-LOCAL"),
    refineRegionGeometry: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  };
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function chooseTemplateFile(fileName: string, contents: string | Uint8Array = "version = 2"): void {
  const bytes = typeof contents === "string" ? new TextEncoder().encode(contents) : contents;
  const input = document.querySelector<HTMLInputElement>('[data-action="template-file"]');
  Object.defineProperty(input, "files", {
    configurable: true,
    value: [{ name: fileName, arrayBuffer: async () => bytes.slice().buffer }],
  });
  input?.dispatchEvent(new Event("change", { bubbles: true }));
}

function installSyntheticMediaDom(): {
  strokeStyles: string[];
  fillStyles: string[];
  drawImage: ReturnType<typeof vi.fn>;
} {
  const strokeStyles: string[] = [];
  const fillStyles: string[] = [];
  const context = {
    setTransform: vi.fn(),
    clearRect: vi.fn(),
    drawImage: vi.fn(),
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    closePath: vi.fn(),
    stroke: vi.fn(),
    fill: vi.fn(),
    arc: vi.fn(),
  };
  Object.defineProperties(context, {
    strokeStyle: {
      set(value: string) {
        strokeStyles.push(value);
      },
    },
    fillStyle: {
      set(value: string) {
        fillStyles.push(value);
      },
    },
  });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context as never);
  vi.spyOn(HTMLCanvasElement.prototype, "clientWidth", "get").mockReturnValue(100);
  vi.spyOn(HTMLCanvasElement.prototype, "clientHeight", "get").mockReturnValue(100);
  vi.spyOn(HTMLCanvasElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 0,
    top: 0,
    right: 100,
    bottom: 100,
    left: 0,
    width: 100,
    height: 100,
    toJSON: () => ({}),
  });
  vi.stubGlobal("ResizeObserver", class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  });
  vi.stubGlobal("Image", class {
    naturalWidth = 100;
    naturalHeight = 100;
    onload: ((event: Event) => void) | null = null;
    onerror: ((event: Event) => void) | null = null;

    set src(_value: string) {
      queueMicrotask(() => this.onload?.(new Event("load")));
    }
  });
  return { strokeStyles, fillStyles, drawImage: context.drawImage };
}

function loadedSyntheticAnnotationApi(
  overrides: Record<string, unknown> = {},
  assets = [{ asset_id: "ASSET-SYNTHETIC", media_kind: "image" as const, size_bytes: 1024 }],
  template = templateV1,
): Record<string, unknown> {
  const scan = {
    campuses: [
      {
        campus_code: "CAMPUS-SYNTHETIC",
        case_count: 1,
        cases: [
          {
            case_id: "CASE-SYNTHETIC",
            date: "2026-08-09",
            assets,
          },
        ],
      },
    ],
  };
  return mockAppApi({
    restoreProjectSession: vi.fn().mockResolvedValue({
      status: "restored",
      scan,
      template: templateDescriptor(template, "a".repeat(64)),
      current_case_id: "CASE-SYNTHETIC",
      current_asset_id: "ASSET-SYNTHETIC",
    }),
    currentLocalDisplayScan: vi.fn().mockResolvedValue({
      campuses: [
        {
          campus_code: "CAMPUS-SYNTHETIC",
          display_name: "合成院区",
          cases: [{ case_id: "CASE-SYNTHETIC", date: "2026-08-09", display_label: "合成病例" }],
        },
      ],
    }),
    representativeAssetIds: vi.fn().mockResolvedValue(["ASSET-SYNTHETIC"]),
    savedRegionsForAsset: vi.fn().mockResolvedValue([syntheticRegion]),
    readImageAsset: vi.fn().mockResolvedValue({
      asset_id: "ASSET-SYNTHETIC",
      mime_type: "image/png",
      data_base64: "synthetic",
    }),
    recordImageDimensions: vi.fn().mockResolvedValue(undefined),
    saveLastPosition: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  });
}

function loadedRefinementApi(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return loadedSyntheticAnnotationApi(overrides);
}

function dispatchPointer(
  canvas: HTMLCanvasElement,
  type: "pointerdown" | "pointermove" | "pointerup" | "pointercancel" | "lostpointercapture",
  x: number,
  y: number,
  options: { buttons?: number; pointerId?: number; button?: number } = {},
): void {
  const event = new MouseEvent(type, {
    bubbles: true,
    button: options.button ?? 0,
    buttons: options.buttons ?? (type === "pointerdown" ? 1 : 0),
    clientX: x,
    clientY: y,
  });
  Object.defineProperties(event, {
    pointerId: { value: options.pointerId ?? 1 },
    pointerType: { value: "mouse" },
  });
  canvas.dispatchEvent(event);
}

function installPointerCaptureMocks(canvas: HTMLCanvasElement): void {
  Object.assign(canvas, {
    setPointerCapture: vi.fn(),
    releasePointerCapture: vi.fn(),
    hasPointerCapture: vi.fn().mockReturnValue(true),
  });
}

async function openSyntheticRefinement(api: Record<string, unknown>): Promise<void> {
  installSyntheticMediaDom();
  window.__GWP_API__ = api as never;
  boot();
  await vi.waitFor(() => {
    const entry = document.querySelector<HTMLButtonElement>('[data-action="refine-region"]');
    expect(entry?.disabled).toBe(false);
  });
  document.querySelector<HTMLButtonElement>('[data-action="refine-region"]')?.click();
  await vi.waitFor(() => {
    expect(document.querySelector('[aria-label="边界精修"]')).not.toBeNull();
  });
}

async function openSyntheticAnnotation(api: Record<string, unknown>): Promise<void> {
  installSyntheticMediaDom();
  window.__GWP_API__ = api as never;
  boot();
  await vi.waitFor(() => {
    expect(document.querySelector('[data-drawing-canvas]')).not.toBeNull();
    expect(document.querySelector('.annotation-state')?.textContent).toContain("可圈选");
  });
}

const chineseToml = `
format = "gwa-label-template"
version = 1
template_id = "template-utf8"

[[fields]]
id = "field_a"
label = "字段甲"
control = "dropdown"
required = true

[[fields.options]]
id = "option_a"
label = "选项甲"
`;

const typicalMojibake = "\u00e6\u00b5\u2039\u00e8\u00af\u2022";
const typicalGbkMojibake = "閫夐」";

describe("drawing geometry", () => {
  it("uses CSS-pixel distance with the documented automatic-close threshold", () => {
    expect(autoCloseThreshold(400)).toBe(12);
    expect(autoCloseThreshold(1200)).toBe(18);
    expect(polygonArea(triangle)).toBeGreaterThan(0.0001);
    expect(canAutoClose(triangle, 17.9, 1200)).toBe(true);
    expect(canAutoClose(triangle, 18.1, 1200)).toBe(false);
  });

  it("rejects too-small and too-short drafts before a dialog can open", () => {
    expect(canAutoClose([{ x: 0.1, y: 0.1 }, { x: 0.2, y: 0.2 }], 0, 500)).toBe(false);
    expect(
      canAutoClose(
        [
          { x: 0.1, y: 0.1 },
          { x: 0.101, y: 0.1 },
          { x: 0.1, y: 0.101 },
        ],
        1,
        500,
      ),
    ).toBe(false);
  });

  it("samples a drawing only while the primary pointer button remains pressed", () => {
    expect(primaryDrawingButtonIsPressed(0)).toBe(false);
    expect(primaryDrawingButtonIsPressed(1)).toBe(true);
    expect(primaryDrawingButtonIsPressed(2)).toBe(false);
    expect(primaryDrawingButtonIsPressed(3)).toBe(true);
  });
});

describe("refinement UI helpers", () => {
  it("keeps brush size in source-image pixels and scales only its canvas display", () => {
    expect(brushDiameterCssPixels(16, 0.5)).toBe(8);
    expect(brushDiameterCssPixels(16, 2)).toBe(32);
    expect(brushDiameterCssPixels(16, Number.NaN)).toBe(0);
  });

  it("requires a selected saved region, loaded image, and representative image", () => {
    expect(canStartRefinement(syntheticRegion, true, true)).toBe(true);
    expect(canStartRefinement(null, true, true)).toBe(false);
    expect(canStartRefinement(syntheticRegion, false, true)).toBe(false);
    expect(canStartRefinement(syntheticRegion, true, false)).toBe(false);
  });

  it("provides a specific Chinese message for an atomic failed stroke", () => {
    expect(refinementFailureMessage("detached")).toContain("必须接触当前区域");
    expect(refinementFailureMessage("hole")).toContain("孔洞");
  });
});

describe("video utility", () => {
  it("formats current time defensively for the local video controls", () => {
    expect(formatVideoTime(0)).toBe("00:00");
    expect(formatVideoTime(65.9)).toBe("01:05");
    expect(formatVideoTime(Number.NaN)).toBe("00:00");
  });
});

describe("keyboard focus guard", () => {
  it("does not let media navigation steal arrows from fields or the label dialog", () => {
    document.body.innerHTML = '<input aria-label="text"><div role="dialog"><button>OK</button></div>';
    const input = document.querySelector("input") as HTMLInputElement;
    const dialogButton = document.querySelector('[role="dialog"] button') as HTMLButtonElement;
    const inputEvent = new KeyboardEvent("keydown", { key: "ArrowRight" });
    const dialogEvent = new KeyboardEvent("keydown", { key: "ArrowLeft" });
    Object.defineProperty(inputEvent, "target", { value: input });
    Object.defineProperty(dialogEvent, "target", { value: dialogButton });

    expect(shouldHandleMediaNavigation(inputEvent)).toBe(false);
    expect(shouldHandleMediaNavigation(dialogEvent)).toBe(false);
    expect(shouldHandleMediaNavigation(new KeyboardEvent("keydown", { key: "ArrowRight" }))).toBe(true);
  });
});

describe("template controls and representative gate", () => {
  const fields: TemplateField[] = [
    {
      id: "field_dropdown",
      label: "Field A",
      control: "dropdown",
      required: true,
      options: [{ id: "option_a", label: "Option A" }],
    },
    {
      id: "field_single",
      label: "Field B",
      control: "single_select",
      required: true,
      options: [{ id: "option_b", label: "Option B" }],
    },
    {
      id: "field_multi",
      label: "Field C",
      control: "multi_select",
      required: true,
      options: [{ id: "option_c", label: "Option C" }],
    },
  ];

  it("requires every dynamic required control before enabling OK", () => {
    expect(labelsMeetRequiredFields(fields, {})).toBe(false);
    expect(
      labelsMeetRequiredFields(fields, {
        field_dropdown: "option_a",
        field_single: "option_b",
        field_multi: [],
      }),
    ).toBe(false);
    expect(
      labelsMeetRequiredFields(fields, {
        field_dropdown: "option_a",
        field_single: "option_b",
        field_multi: ["option_c"],
      }),
    ).toBe(true);
  });

  it("permits drawing only on a representative image with a loaded template", () => {
    expect(canAnnotate(false, false)).toBe(false);
    expect(canAnnotate(true, false)).toBe(false);
    expect(canAnnotate(false, true)).toBe(false);
    expect(canAnnotate(true, true)).toBe(true);
  });

  it("allows TOML replacement while keeping TXT as first-import compatibility only", () => {
    expect(templateImportAction(false, "label-template-v1.toml")).toBe("install_toml");
    expect(templateImportAction(true, "label-template-v2.TOML")).toBe("replace_toml");
    expect(templateImportAction(false, "legacy-label.txt")).toBe("install_txt");
    expect(templateImportAction(true, "legacy-label.txt")).toBe("unsupported");
    expect(templateImportAction(false, "labels.json")).toBe("unsupported");
  });
});

describe("template UTF-8 import", () => {
  it("decodes a valid Chinese UTF-8 TOML template without altering its labels", () => {
    expect(decodeUtf8Template(new TextEncoder().encode(chineseToml))).toBe(chineseToml);
  });

  it("rejects invalid UTF-8 bytes before IPC", () => {
    expect(() => decodeUtf8Template(new Uint8Array([0xff, 0xfe]))).toThrow(TEMPLATE_UTF8_IMPORT_ERROR);
  });

  it.each(["字段\uFFFD", "字段\uE000", typicalMojibake, typicalGbkMojibake])(
    "rejects damaged template text %j",
    (damagedLabel) => {
      const damagedToml = chineseToml.replace("字段甲", damagedLabel);
      expect(() => decodeUtf8Template(new TextEncoder().encode(damagedToml))).toThrow(
        TEMPLATE_UTF8_IMPORT_ERROR,
      );
    },
  );

  it("identifies a recovered legacy template with explicit damage without treating it as a new import", () => {
    expect(templateHasEncodingDamage(legacyDamagedTemplate)).toBe(true);
    expect(templateHasEncodingDamage(templateV1)).toBe(false);
    expect(templateAllowsNewAnnotations(legacyDamagedTemplate)).toBe(false);
    expect(templateAllowsNewAnnotations(templateV1)).toBe(true);
  });
});

describe("custom drawing crosshair", () => {
  it("appears only for a mouse over an annotation-ready image outside pan and dialog states", () => {
    expect(shouldShowCustomCrosshair(true, true, false, true, true, false)).toBe(true);
    expect(shouldShowCustomCrosshair(false, true, false, true, true, false)).toBe(false);
    expect(shouldShowCustomCrosshair(true, false, false, true, true, false)).toBe(false);
    expect(shouldShowCustomCrosshair(true, true, true, true, true, false)).toBe(false);
    expect(shouldShowCustomCrosshair(true, true, false, false, true, false)).toBe(false);
    expect(shouldShowCustomCrosshair(true, true, false, true, false, false)).toBe(false);
    expect(shouldShowCustomCrosshair(true, true, false, true, true, true)).toBe(false);
  });

  it("hides the cursor state when pointer capture stops", () => {
    const stopped = clearCrosshairPointerState({ pointerInCanvas: true, x: 320, y: 240 });

    expect(stopped).toEqual({ pointerInCanvas: false, x: 320, y: 240 });
    expect(shouldShowCustomCrosshair(true, true, false, stopped.pointerInCanvas, true, false)).toBe(false);
  });

  it("renders a test-addressable non-interactive cursor element with stable coordinates", () => {
    document.body.innerHTML = customCrosshairMarkup(true, 42.4, 64.6);
    const cursor = document.querySelector<HTMLElement>("[data-drawing-crosshair]");

    expect(cursor?.dataset.drawingCrosshair).toBe("visible");
    expect(cursor?.getAttribute("aria-hidden")).toBe("true");
    expect(cursor?.classList.contains("drawing-crosshair")).toBe(true);
    expect(cursor?.style.pointerEvents).toBe("none");
    expect(cursor?.style.zIndex).toBe("6");
    expect(cursor?.style.getPropertyValue("--crosshair-x")).toBe("42px");
    expect(cursor?.style.getPropertyValue("--crosshair-y")).toBe("65px");
  });
});

describe("local shell", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="app"></div>';
  });

  afterEach(() => {
    delete window.__GWP_API__;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders an offline local status without a built-in medical template", async () => {
    boot();

    expect(document.querySelector('[data-testid="offline-status"]')?.textContent).toBe("本地离线");
    expect(document.body.textContent).toContain("未加载。请选择 label-template-v1.toml。");
    expect(document.querySelector<HTMLButtonElement>('[data-action="storage-location"]')).not.toBeNull();
    await vi.waitFor(() => {
      expect(document.querySelector<HTMLInputElement>('[data-action="template-file"]')?.disabled).toBe(false);
      expect(document.querySelector<HTMLButtonElement>('[data-action="forget-position"]')?.disabled).toBe(false);
    });
  });

  it("keeps the Stage 4 three-region, selection, disabled, dialog, and canvas-size contracts", async () => {
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi());

    const layout = document.querySelector<HTMLElement>('[data-workspace-layout="three-region"]');
    expect(document.querySelector(".workbench")?.getAttribute("data-visual-density")).toBe("compact-clinical");
    expect(layout).not.toBeNull();
    expect(layout?.querySelectorAll(":scope > [data-workspace-zone]")).toHaveLength(3);
    expect(layout?.querySelector('[data-workspace-zone="case-navigation"]')).not.toBeNull();
    expect(layout?.querySelector('[data-workspace-zone="media-canvas"]')).not.toBeNull();
    expect(layout?.querySelector('[data-workspace-zone="annotation-inspector"]')).not.toBeNull();

    const mediaPanel = layout?.querySelector<HTMLElement>('[data-workspace-zone="media-canvas"]');
    const canvasFrame = mediaPanel?.querySelector<HTMLElement>("[data-canvas-frame]");
    expect(mediaPanel?.children).toHaveLength(3);
    expect(mediaPanel?.children[0]?.classList.contains("media-toolbar")).toBe(true);
    expect(mediaPanel?.children[1]).toBe(canvasFrame);
    expect(mediaPanel?.children[2]?.classList.contains("media-strip")).toBe(true);

    expect(document.querySelector('.case-item.selected')?.getAttribute("aria-selected")).toBe("true");
    expect(document.querySelector('.media-item.current')?.getAttribute("data-selected")).toBe("true");
    expect(document.querySelector('.media-item.current [data-action="asset"]')?.getAttribute("aria-current")).toBe("true");
    expect(document.querySelector('.region-item.selected')?.getAttribute("data-selected")).toBe("true");
    expect(document.querySelector('.region-item.selected [data-region-id]')?.getAttribute("aria-pressed")).toBe("true");
    expect(document.querySelector<HTMLButtonElement>('[data-action="previous"]')?.disabled).toBe(true);
    expect(document.querySelector<HTMLButtonElement>('[data-action="next"]')?.disabled).toBe(true);
    expect(document.querySelector('[data-canvas-frame]')?.getAttribute("data-canvas-size-contract")).toBe("contained");

    document.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.click();
    await vi.waitFor(() => expect(document.querySelector('[data-dialog-layer="modal"]')).not.toBeNull());
    expect(document.querySelector('[data-edit-dialog]')?.getAttribute("data-dialog-kind")).toBe("edit-region");
    expect(document.querySelector('[data-edit-dialog]')?.getAttribute("aria-modal")).toBe("true");
  });

  it("shows the selected-region refinement entry and source-pixel brush controls", async () => {
    await openSyntheticRefinement(loadedRefinementApi());

    const brush = document.querySelector<HTMLSelectElement>('[data-action="refinement-brush"]');
    expect(document.body.textContent).toContain("精修边界");
    expect(document.body.textContent).toContain("笔刷粗细");
    expect(document.body.textContent).toContain("原图像素");
    expect(Array.from(brush?.options ?? [], (option) => option.value)).toEqual(["4", "8", "16", "24"]);
    expect(brush?.value).toBe("8");
    expect(document.querySelector('[data-drawing-crosshair]')).toBeNull();
    expect(document.querySelector('[data-canvas-frame]')?.classList.contains("refinement-mode")).toBe(true);
  });

  it("saves expected and current polygons without retransmitting labels", async () => {
    const refineRegionGeometry = vi.fn().mockResolvedValue(undefined);
    const api = loadedRefinementApi({ refineRegionGeometry });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticRefinement(api);

    const canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(canvas);
    dispatchPointer(canvas, "pointerdown", 22, 50);
    dispatchPointer(canvas, "pointerup", 5, 50);

    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(false);
    });
    document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.click();

    await vi.waitFor(() => expect(refineRegionGeometry).toHaveBeenCalledOnce());
    const request = refineRegionGeometry.mock.calls[0][0] as Record<string, unknown>;
    expect(request).toMatchObject({
      case_id: "CASE-SYNTHETIC",
      asset_id: "ASSET-SYNTHETIC",
      region_id: "REGION-SYNTHETIC",
      expected_polygon_norm: syntheticRegion.polygon_norm,
    });
    const expected = refinePolygonWithStroke(
      syntheticRegion.polygon_norm,
      100,
      100,
      [{ x: 0.22, y: 0.5 }, { x: 0.05, y: 0.5 }],
      8,
    );
    expect(expected.ok).toBe(true);
    if (!expected.ok) throw new Error(`Expected synthetic refinement to succeed: ${expected.reason}`);
    expect(request.polygon_norm).toEqual(expected.polygon);
    expect(request).not.toHaveProperty("labels");
  });

  it("cancels by button or Escape without calling the refinement API", async () => {
    const refineRegionGeometry = vi.fn().mockResolvedValue(undefined);
    const api = loadedRefinementApi({ refineRegionGeometry });
    await openSyntheticRefinement(api);

    document.querySelector<HTMLButtonElement>('[data-action="refinement-cancel"]')?.click();
    expect(document.querySelector('[aria-label="边界精修"]')).toBeNull();
    document.querySelector<HTMLButtonElement>('[data-action="refine-region"]')?.click();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(document.querySelector('[aria-label="边界精修"]')).toBeNull();
    expect(refineRegionGeometry).not.toHaveBeenCalled();
  });

  it("undoes the latest stroke before applying and saving a different stroke", async () => {
    const refineRegionGeometry = vi.fn().mockResolvedValue(undefined);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticRefinement(loadedRefinementApi({ refineRegionGeometry }));

    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(canvas);
    dispatchPointer(canvas, "pointerdown", 22, 50);
    dispatchPointer(canvas, "pointerup", 5, 50);
    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-undo"]')?.disabled).toBe(false);
    });
    document.querySelector<HTMLButtonElement>('[data-action="refinement-undo"]')?.click();
    expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(true);

    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(canvas);
    dispatchPointer(canvas, "pointerdown", 78, 50);
    dispatchPointer(canvas, "pointerup", 95, 50);
    document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.click();
    await vi.waitFor(() => expect(refineRegionGeometry).toHaveBeenCalledOnce());

    const expected = refinePolygonWithStroke(
      syntheticRegion.polygon_norm,
      100,
      100,
      [{ x: 0.78, y: 0.5 }, { x: 0.95, y: 0.5 }],
      8,
    );
    expect(expected.ok).toBe(true);
    if (!expected.ok) throw new Error(`Expected synthetic refinement to succeed: ${expected.reason}`);
    expect(refineRegionGeometry.mock.calls[0][0].polygon_norm).toEqual(expected.polygon);
  });

  it("does not discard a completed refinement stroke when re-entry is triggered programmatically", async () => {
    await openSyntheticRefinement(loadedRefinementApi());

    const canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(canvas);
    dispatchPointer(canvas, "pointerdown", 22, 50);
    dispatchPointer(canvas, "pointerup", 5, 50);
    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(false);
    });

    document.querySelector<HTMLButtonElement>('[data-action="refine-region"]')?.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(false);
    expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-undo"]')?.disabled).toBe(false);
  });

  it("keeps completed draft edits across pointer termination and a save conflict", async () => {
    const refineRegionGeometry = vi.fn().mockRejectedValue({ code: "REGION_GEOMETRY_CONFLICT" });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticRefinement(loadedRefinementApi({ refineRegionGeometry }));

    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(canvas);
    dispatchPointer(canvas, "pointerdown", 22, 50);
    dispatchPointer(canvas, "pointerup", 22, 50);
    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(false);
    });

    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(canvas);
    dispatchPointer(canvas, "pointerdown", 24, 50);
    dispatchPointer(canvas, "pointercancel", 24, 50);
    dispatchPointer(canvas, "pointerdown", 24, 50);
    dispatchPointer(canvas, "lostpointercapture", 24, 50);
    expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(false);

    document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.click();
    await vi.waitFor(() => {
      expect(document.querySelector(".notice")?.textContent).toContain("请刷新后重试");
    });
    expect(document.querySelector('[aria-label="边界精修"]')).not.toBeNull();
    expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(false);
  });

  it("keeps the draft unchanged and reports Chinese guidance when a stroke fails", async () => {
    await openSyntheticRefinement(loadedRefinementApi());

    const brush = document.querySelector<HTMLSelectElement>('[data-action="refinement-brush"]') as HTMLSelectElement;
    brush.value = "4";
    brush.dispatchEvent(new Event("change", { bubbles: true }));
    const canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(canvas);
    dispatchPointer(canvas, "pointerdown", 5, 5);
    dispatchPointer(canvas, "pointerup", 5, 5);

    await vi.waitFor(() => {
      expect(document.querySelector(".notice")?.textContent).toContain("必须接触当前区域");
    });
    expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-save"]')?.disabled).toBe(true);
    expect(document.querySelector<HTMLButtonElement>('[data-action="refinement-undo"]')?.disabled).toBe(true);
  });

  it("confirms and invokes the replacement API when a template is already active", async () => {
    const replaceFixedTomlTemplate = vi.fn().mockResolvedValue(templateDescriptor(templateV2, "b".repeat(64)));
    window.__GWP_API__ = mockAppApi({
      restoreProjectSession: vi.fn().mockResolvedValue(restoredWithTemplate(templateV1, "a".repeat(64))),
      replaceFixedTomlTemplate,
    }) as never;
    vi.spyOn(window, "confirm").mockReturnValue(true);
    boot();
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("当前模板：template-test v1");
    });

    const input = document.querySelector<HTMLInputElement>('[data-action="template-file"]');
    expect(input?.getAttribute("aria-label")).toBe("更换标签模板");
    chooseTemplateFile("label-template-v2.toml");

    await vi.waitFor(() => {
      expect(replaceFixedTomlTemplate).toHaveBeenCalledWith("version = 2");
      expect(document.body.textContent).toContain("当前模板：template-test v2");
      expect(document.body.textContent).toContain("标签模板已更换");
    });
  });

  it("does not send invalid UTF-8 template bytes to the backend", async () => {
    const installFixedTomlTemplate = vi.fn();
    window.__GWP_API__ = mockAppApi({ installFixedTomlTemplate }) as never;
    boot();

    chooseTemplateFile("label-template-v1.toml", new Uint8Array([0xff]));

    await vi.waitFor(() => {
      expect(installFixedTomlTemplate).not.toHaveBeenCalled();
      expect(document.body.textContent).toContain(TEMPLATE_UTF8_IMPORT_ERROR);
    });
  });

  it("warns about a restored legacy damaged template while retaining its replacement control", async () => {
    window.__GWP_API__ = mockAppApi({
      restoreProjectSession: vi.fn().mockResolvedValue(restoredWithTemplate(legacyDamagedTemplate, "a".repeat(64))),
    }) as never;
    boot();

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain(LEGACY_TEMPLATE_ENCODING_WARNING);
      const input = document.querySelector<HTMLInputElement>('[data-action="template-file"]');
      expect(input?.getAttribute("aria-label")).toBe("更换标签模板");
      expect(input?.disabled).toBe(false);
    });
  });

  it("warns after selecting a root with a legacy damaged template while retaining replacement", async () => {
    const chooseAndScanRoot = vi.fn().mockResolvedValue({ campuses: [] });
    const currentFixedTemplate = vi.fn().mockResolvedValue(templateDescriptor(legacyDamagedTemplate, "a".repeat(64)));
    window.__GWP_API__ = mockAppApi({ chooseAndScanRoot, currentFixedTemplate }) as never;
    boot();

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("请选择本地数据根目录。");
    });
    document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.click();

    await vi.waitFor(() => {
      expect(chooseAndScanRoot).toHaveBeenCalledOnce();
      expect(currentFixedTemplate).toHaveBeenCalledOnce();
      expect(document.body.textContent).toContain(LEGACY_TEMPLATE_ENCODING_WARNING);
      const input = document.querySelector<HTMLInputElement>('[data-action="template-file"]');
      expect(input?.getAttribute("aria-label")).toBe("更换标签模板");
      expect(input?.disabled).toBe(false);
    });
  });

  it("leaves the active template unchanged when replacement is cancelled", async () => {
    const replaceFixedTomlTemplate = vi.fn();
    window.__GWP_API__ = mockAppApi({
      restoreProjectSession: vi.fn().mockResolvedValue(restoredWithTemplate(templateV1, "a".repeat(64))),
      replaceFixedTomlTemplate,
    }) as never;
    vi.spyOn(window, "confirm").mockReturnValue(false);
    boot();
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("当前模板：template-test v1");
    });

    chooseTemplateFile("label-template-v2.toml");

    await Promise.resolve();
    expect(replaceFixedTomlTemplate).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("当前模板：template-test v1");
  });

  it("reconciles to the backend template when replacement reports an error after switching it", async () => {
    const replacementError = { code: "TEMPLATE_REPLACEMENT_IDENTITY_CONFLICT" };
    const replaceFixedTomlTemplate = vi.fn().mockRejectedValue(replacementError);
    const currentFixedTemplate = vi.fn().mockResolvedValue(templateDescriptor(templateV2, "b".repeat(64)));
    window.__GWP_API__ = mockAppApi({
      restoreProjectSession: vi.fn().mockResolvedValue(restoredWithTemplate(templateV1, "a".repeat(64))),
      replaceFixedTomlTemplate,
      currentFixedTemplate,
    }) as never;
    vi.spyOn(window, "confirm").mockReturnValue(true);
    boot();
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("当前模板：template-test v1");
    });

    chooseTemplateFile("label-template-v2.toml");

    await vi.waitFor(() => {
      expect(currentFixedTemplate).toHaveBeenCalledOnce();
      expect(document.body.textContent).toContain("当前模板：template-test v2");
      expect(document.body.textContent).toContain("已根据后端当前模板刷新界面。");
      expect(document.querySelector(".notice")?.classList.contains("error")).toBe(true);
    });
  });

  it("clears the active template when failed replacement reload reports none", async () => {
    const replaceFixedTomlTemplate = vi.fn().mockRejectedValue({ code: "FIXED_TEMPLATE_MISSING" });
    const currentFixedTemplate = vi.fn().mockResolvedValue(null);
    window.__GWP_API__ = mockAppApi({
      restoreProjectSession: vi.fn().mockResolvedValue(restoredWithTemplate(templateV1, "a".repeat(64))),
      replaceFixedTomlTemplate,
      currentFixedTemplate,
    }) as never;
    vi.spyOn(window, "confirm").mockReturnValue(true);
    boot();
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("当前模板：template-test v1");
    });

    chooseTemplateFile("label-template-v2.toml");

    await vi.waitFor(() => {
      expect(currentFixedTemplate).toHaveBeenCalledOnce();
      expect(document.body.textContent).toContain("未加载。请选择 label-template-v1.toml。");
      expect(document.body.textContent).toContain("后端当前未加载标签模板。");
    });
  });

  it("retains the displayed template and reports an unrecoverable reload failure", async () => {
    const replaceFixedTomlTemplate = vi.fn().mockRejectedValue({ code: "FIXED_TEMPLATE_MISSING" });
    const currentFixedTemplate = vi.fn().mockRejectedValue({ code: "UNEXPECTED_ERROR" });
    window.__GWP_API__ = mockAppApi({
      restoreProjectSession: vi.fn().mockResolvedValue(restoredWithTemplate(templateV1, "a".repeat(64))),
      replaceFixedTomlTemplate,
      currentFixedTemplate,
    }) as never;
    vi.spyOn(window, "confirm").mockReturnValue(true);
    boot();
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("当前模板：template-test v1");
    });

    chooseTemplateFile("label-template-v2.toml");

    await vi.waitFor(() => {
      expect(currentFixedTemplate).toHaveBeenCalledOnce();
      expect(document.body.textContent).toContain("当前模板：template-test v1");
      expect(document.body.textContent).toContain("未能读取后端当前模板，界面保留原模板。");
    });
  });

  it("shows the backend-provided project storage location", async () => {
    const projectStorageLocation = vi
      .fn()
      .mockResolvedValue("D:\\Apps\\GWP Annotator Data\\PROJECT-LOCAL");
    window.__GWP_API__ = mockAppApi({ projectStorageLocation }) as never;
    boot();

    document
      .querySelector<HTMLButtonElement>('[data-action="storage-location"]')
      ?.click();

    await vi.waitFor(() => {
      expect(projectStorageLocation).toHaveBeenCalledOnce();
      expect(document.querySelector(".storage-location output")?.textContent).toBe(
        "D:\\Apps\\GWP Annotator Data\\PROJECT-LOCAL",
      );
    });
  });

  it("reloads project data using the backend-restored case and asset IDs", async () => {
    const assets = [
      { asset_id: "ASSET-SYNTHETIC", media_kind: "image" as const, size_bytes: 1024 },
      { asset_id: "ASSET-SECOND", media_kind: "image" as const, size_bytes: 2048 },
    ];
    const scan = {
      campuses: [{
        campus_code: "CAMPUS-SYNTHETIC",
        case_count: 1,
        cases: [{ case_id: "CASE-SYNTHETIC", date: "2026-08-09", assets }],
      }],
    };
    const restoredSecondRegion = { ...syntheticRegion, asset_id: "ASSET-SECOND" };
    const reloadRestore = deferred<Record<string, unknown>>();
    const restoreProjectSession = vi.fn()
      .mockResolvedValueOnce({
        status: "restored",
        scan,
        template: templateDescriptor(templateV1, "a".repeat(64)),
        current_case_id: "CASE-SYNTHETIC",
        current_asset_id: "ASSET-SYNTHETIC",
      })
      .mockImplementationOnce(() => reloadRestore.promise);
    const reloadResult = {
        status: "restored",
        scan,
        template: templateDescriptor(templateV1, "a".repeat(64)),
        current_case_id: "CASE-SYNTHETIC",
        current_asset_id: "ASSET-SECOND",
      };
    const savedRegionsForAsset = vi.fn()
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([restoredSecondRegion]);
    installSyntheticMediaDom();
    window.__GWP_API__ = loadedSyntheticAnnotationApi(
      { restoreProjectSession, savedRegionsForAsset },
      assets,
    ) as never;
    boot();

    await vi.waitFor(() => expect(document.body.textContent).toContain("1 / 2"));
    document.querySelector<HTMLButtonElement>('[data-action="reload-project"]')?.click();
    await vi.waitFor(() => {
      expect(restoreProjectSession).toHaveBeenCalledTimes(2);
      expect(document.querySelector<HTMLButtonElement>('[data-action="reload-project"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.disabled).toBe(true);
      expect(document.body.textContent).toContain("正在重新加载…");
    });
    reloadRestore.resolve(reloadResult);

    await vi.waitFor(() => {
      expect(savedRegionsForAsset).toHaveBeenLastCalledWith("CASE-SYNTHETIC", "ASSET-SECOND");
      expect(document.body.textContent).toContain("2 / 2");
      expect(document.body.textContent).toContain("项目数据已重新加载。当前图片已加载 1 个已保存区域。");
    });
  });

  it("reapplies the backend-restored location after choosing the same root", async () => {
    const assets = [
      { asset_id: "ASSET-SYNTHETIC", media_kind: "image" as const, size_bytes: 1024 },
      { asset_id: "ASSET-SECOND", media_kind: "image" as const, size_bytes: 2048 },
    ];
    const scan = {
      campuses: [{
        campus_code: "CAMPUS-SYNTHETIC",
        case_count: 1,
        cases: [{ case_id: "CASE-SYNTHETIC", date: "2026-08-09", assets }],
      }],
    };
    const chooseAndScanRoot = vi.fn().mockResolvedValue(scan);
    const restoreProjectSession = vi.fn()
      .mockResolvedValueOnce({
        status: "no_session",
        scan: null,
        template: null,
        current_case_id: null,
        current_asset_id: null,
      })
      .mockResolvedValueOnce({
        status: "restored",
        scan,
        template: templateDescriptor(templateV1, "a".repeat(64)),
        current_case_id: "CASE-SYNTHETIC",
        current_asset_id: "ASSET-SECOND",
      });
    const savedRegionsForAsset = vi.fn().mockResolvedValue([]);
    installSyntheticMediaDom();
    window.__GWP_API__ = loadedSyntheticAnnotationApi(
      { chooseAndScanRoot, restoreProjectSession, savedRegionsForAsset },
      assets,
    ) as never;
    boot();

    await vi.waitFor(() => expect(document.body.textContent).toContain("请选择本地数据根目录。"));
    document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.click();

    await vi.waitFor(() => {
      expect(chooseAndScanRoot).toHaveBeenCalledOnce();
      expect(restoreProjectSession).toHaveBeenCalledTimes(2);
      expect(savedRegionsForAsset).toHaveBeenCalledWith("CASE-SYNTHETIC", "ASSET-SECOND");
      expect(document.body.textContent).toContain("2 / 2");
      expect(document.body.textContent).toContain("已加载所选数据目录。");
    });
  });

  it("keeps a user-selected root when the older startup restore resolves later", async () => {
    const startupRestore = deferred<Record<string, unknown>>();
    const selectionRestore = deferred<Record<string, unknown>>();
    const oldScan = {
      campuses: [{
        campus_code: "CAMPUS-OLD",
        case_count: 1,
        cases: [{
          case_id: "CASE-OLD",
          date: "2026-08-01",
          assets: [{ asset_id: "ASSET-OLD", media_kind: "image" as const, size_bytes: 512 }],
        }],
      }],
    };
    const selectedScan = {
      campuses: [{
        campus_code: "CAMPUS-SELECTED",
        case_count: 1,
        cases: [{
          case_id: "CASE-SELECTED",
          date: "2026-08-09",
          assets: [{ asset_id: "ASSET-SELECTED", media_kind: "image" as const, size_bytes: 1024 }],
        }],
      }],
    };
    const chooseAndScanRoot = vi.fn().mockResolvedValue(selectedScan);
    const restoreProjectSession = vi.fn()
      .mockImplementationOnce(() => startupRestore.promise)
      .mockImplementationOnce(() => selectionRestore.promise);
    const representativeAssetIds = vi.fn().mockResolvedValue([]);
    installSyntheticMediaDom();
    window.__GWP_API__ = loadedSyntheticAnnotationApi({
      chooseAndScanRoot,
      restoreProjectSession,
      representativeAssetIds,
      savedRegionsForAsset: vi.fn().mockResolvedValue([]),
    }) as never;
    boot();

    expect(document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.disabled).toBe(false);
    expect(document.querySelector<HTMLButtonElement>('[data-action="reload-project"]')?.disabled).toBe(true);
    expect(document.body.textContent).toContain("正在恢复项目…");
    document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.click();
    await vi.waitFor(() => {
      expect(restoreProjectSession).toHaveBeenCalledTimes(2);
      expect(document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.disabled).toBe(true);
      expect(document.body.textContent).toContain("正在选择数据目录…");
    });

    selectionRestore.resolve({
      status: "restored",
      scan: selectedScan,
      template: templateDescriptor(templateV1, "b".repeat(64)),
      current_case_id: "CASE-SELECTED",
      current_asset_id: "ASSET-SELECTED",
    });
    await vi.waitFor(() => {
      expect(document.querySelector('[data-case-id="CASE-SELECTED"]')?.getAttribute("aria-selected")).toBe("true");
      expect(document.body.textContent).toContain("已加载所选数据目录。");
      expect(document.querySelector<HTMLButtonElement>('[data-action="reload-project"]')?.disabled).toBe(false);
    });

    startupRestore.resolve({
      status: "restored",
      scan: oldScan,
      template: templateDescriptor(templateV1, "a".repeat(64)),
      current_case_id: "CASE-OLD",
      current_asset_id: "ASSET-OLD",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(document.querySelector('[data-case-id="CASE-SELECTED"]')?.getAttribute("aria-selected")).toBe("true");
    expect(document.querySelector('[data-case-id="CASE-OLD"]')).toBeNull();
    expect(document.body.textContent).toContain("已加载所选数据目录。");
    expect(representativeAssetIds).not.toHaveBeenCalledWith("CASE-OLD");
  });

  it("explains when project reload has no saved data root", async () => {
    const restoreProjectSession = vi.fn().mockResolvedValue({
      status: "no_session",
      scan: null,
      template: null,
      current_case_id: null,
      current_asset_id: null,
    });
    window.__GWP_API__ = mockAppApi({ restoreProjectSession }) as never;
    boot();

    await vi.waitFor(() => expect(document.body.textContent).toContain("请选择本地数据根目录。"));
    document.querySelector<HTMLButtonElement>('[data-action="reload-project"]')?.click();
    await vi.waitFor(() => {
      expect(restoreProjectSession).toHaveBeenCalledTimes(2);
      expect(document.body.textContent).toContain("尚未选择数据根目录，请先选择。");
    });
  });

  it("renders and activates a separate representative-image button", async () => {
    const selectRepresentative = vi.fn().mockResolvedValue({
      case_id: "CASE-SYNTHETIC",
      asset_id: "ASSET-SYNTHETIC",
      representative_count: 1,
    });
    installSyntheticMediaDom();
    window.__GWP_API__ = loadedSyntheticAnnotationApi({
      representativeAssetIds: vi.fn().mockResolvedValue([]),
      savedRegionsForAsset: vi.fn().mockResolvedValue([]),
      selectRepresentative,
    }) as never;
    boot();

    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>('[data-action="asset"]')?.textContent).toContain("图片 01");
      expect(document.querySelector<HTMLButtonElement>('[data-action="asset"]')?.getAttribute("aria-current")).toBe("true");
      expect(document.querySelector<HTMLButtonElement>('[data-action="representative"]')?.textContent).toBe("设为代表");
    });
    document.querySelector<HTMLButtonElement>('[data-action="representative"]')?.click();

    await vi.waitFor(() => {
      expect(selectRepresentative).toHaveBeenCalledWith("CASE-SYNTHETIC", "ASSET-SYNTHETIC");
      expect(document.querySelector<HTMLButtonElement>('[data-action="representative"]')?.textContent).toBe("取消代表");
      expect(document.querySelector(".annotation-state")?.textContent).toContain("可圈选");
      expect(document.querySelector(".notice")?.textContent).toContain("已选择为代表图");
    });
  });

  it("keeps label edits open for radio changes and only cancels from explicit dialog exits", async () => {
    const updateRegionLabels = vi.fn().mockResolvedValue(undefined);
    const savedRegionsForAsset = vi.fn().mockResolvedValue([editableSyntheticRegion]);
    await openSyntheticAnnotation(
      loadedSyntheticAnnotationApi({ updateRegionLabels, savedRegionsForAsset }),
    );

    document.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.click();
    await vi.waitFor(() => expect(document.querySelector('[data-edit-dialog]')).not.toBeNull());
    expect(document.activeElement).toBe(document.querySelector('[data-edit-dialog]'));

    const betaOption = document.querySelector<HTMLInputElement>('[data-edit-dialog] input[value="option_beta"]') as HTMLInputElement;
    betaOption.focus();
    betaOption.click();
    await vi.waitFor(() => {
      expect(document.querySelector('[data-edit-dialog]')).not.toBeNull();
      expect(betaOption.isConnected).toBe(true);
      expect(betaOption.checked).toBe(true);
      expect(document.activeElement).toBe(betaOption);
    });
    expect(updateRegionLabels).not.toHaveBeenCalled();

    document.querySelector<HTMLElement>('[data-edit-dialog]')?.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    expect(document.querySelector('[data-edit-dialog]')).not.toBeNull();

    document.querySelector<HTMLFormElement>('[data-edit-label-form]')?.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(updateRegionLabels).toHaveBeenCalledOnce());
    expect(updateRegionLabels).toHaveBeenCalledWith(
      expect.objectContaining({
        case_id: "CASE-SYNTHETIC",
        asset_id: "ASSET-SYNTHETIC",
        region_id: "REGION-SYNTHETIC",
        labels: { field_editable: "option_beta" },
      }),
    );
    await vi.waitFor(() => expect(document.querySelector('[data-edit-dialog]')).toBeNull());

    document.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.click();
    await vi.waitFor(() => expect(document.querySelector('[data-edit-dialog]')).not.toBeNull());
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(document.querySelector('[data-edit-dialog]')).toBeNull();

    document.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.click();
    await vi.waitFor(() => expect(document.querySelector('[data-edit-dialog]')).not.toBeNull());
    document.querySelector<HTMLElement>(".modal-backdrop")?.dispatchEvent(
      new MouseEvent("click", { bubbles: true }),
    );
    expect(document.querySelector('[data-edit-dialog]')).toBeNull();
  });

  it("renders all official serosa labels as flat radio choices for creation and editing", async () => {
    const updateRegionLabels = vi.fn().mockResolvedValue(undefined);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi(
      {
        updateRegionLabels,
        savedRegionsForAsset: vi.fn().mockResolvedValue([officialSerosaRegion]),
      },
      [{ asset_id: "ASSET-SYNTHETIC", media_kind: "image", size_bytes: 1024 }],
      officialSerosaTemplate,
    ));

    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 20, 20, { buttons: 0 });
    await vi.waitFor(() => expect(document.querySelector("[data-label-form]")).not.toBeNull());

    const expectFlatOfficialControls = (form: HTMLFormElement): void => {
      expect(form.querySelectorAll("select")).toHaveLength(0);
      expect(form.querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
      expect(form.querySelectorAll('input[type="radio"]')).toHaveLength(12);
      expect(form.querySelectorAll('input[name="serosal_classification"]')).toHaveLength(6);
      expect(form.querySelectorAll('input[name="serosal_white_plaque_classification"]')).toHaveLength(3);
      expect(form.querySelectorAll('input[name="serosal_white_plaque_confidence"]')).toHaveLength(3);
    };
    expectFlatOfficialControls(document.querySelector<HTMLFormElement>("[data-label-form]") as HTMLFormElement);

    document.querySelector<HTMLButtonElement>('button[data-action="cancel-draft"]')?.click();
    await vi.waitFor(() => expect(document.querySelector("[data-label-form]")).toBeNull());
    document.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.click();
    await vi.waitFor(() => expect(document.querySelector("[data-edit-label-form]")).not.toBeNull());

    const editForm = document.querySelector<HTMLFormElement>("[data-edit-label-form]") as HTMLFormElement;
    expectFlatOfficialControls(editForm);
    expect(editForm.querySelectorAll('input[type="radio"]:checked')).toHaveLength(3);
    editForm.querySelector<HTMLInputElement>('input[value="reactive"]')?.click();
    editForm.querySelector<HTMLInputElement>('input[value="protruding"]')?.click();
    editForm.querySelector<HTMLInputElement>('input[value="confidence_100"]')?.click();
    expect(document.querySelector("[data-edit-label-form]")).toBe(editForm);
    editForm.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    await vi.waitFor(() => expect(updateRegionLabels).toHaveBeenCalledOnce());
    expect(updateRegionLabels).toHaveBeenCalledWith(expect.objectContaining({
      template_id: officialSerosaTemplate.template_id,
      template_version: officialSerosaTemplate.version,
      labels: {
        serosal_classification: "reactive",
        serosal_white_plaque_classification: "protruding",
        serosal_white_plaque_confidence: "confidence_100",
      },
    }));
    await vi.waitFor(() => expect(document.querySelector("[data-edit-label-form]")).toBeNull());
  });

  it("locks edit-label submission synchronously and updates the region once", async () => {
    const pendingUpdate = deferred<void>();
    const updateRegionLabels = vi.fn().mockImplementation(() => pendingUpdate.promise);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({
      updateRegionLabels,
      savedRegionsForAsset: vi.fn().mockResolvedValue([editableSyntheticRegion]),
    }));

    document.querySelector<HTMLButtonElement>('[data-action="edit-region"]')?.click();
    await vi.waitFor(() => expect(document.querySelector('[data-edit-dialog]')).not.toBeNull());
    const form = document.querySelector<HTMLFormElement>("[data-edit-label-form]") as HTMLFormElement;
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    expect(updateRegionLabels).toHaveBeenCalledOnce();
    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>('[data-edit-label-form] button[type="submit"]')?.disabled).toBe(true);
      expect(document.body.textContent).toContain("正在保存…");
      expect(Array.from(document.querySelectorAll<HTMLButtonElement>('button[data-action="cancel-edit"]')).every((button) => button.disabled)).toBe(true);
      expect(document.querySelector<HTMLInputElement>('[data-edit-dialog] input[value="option_alpha"]')?.disabled).toBe(true);
    });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(document.querySelector('[data-edit-dialog]')).not.toBeNull();

    pendingUpdate.resolve(undefined);
    await vi.waitFor(() => expect(document.querySelector('[data-edit-dialog]')).toBeNull());
    expect(updateRegionLabels).toHaveBeenCalledOnce();
  });

  it("continues a free lasso after mouse release and opens labels when it returns to its start", async () => {
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ savedRegionsForAsset: vi.fn().mockResolvedValue([]) }));

    const startCanvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(startCanvas);
    dispatchPointer(startCanvas, "pointerdown", 20, 20);
    dispatchPointer(startCanvas, "pointerup", 20, 20);

    const drawingCanvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(drawingCanvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(drawingCanvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(drawingCanvas, "pointermove", 20, 20, { buttons: 0 });

    await vi.waitFor(() => {
      expect(document.querySelector('[data-label-dialog]')).not.toBeNull();
      expect(document.body.textContent).toContain("填写区域标签");
    });
  });

  it("locks a new-region submit synchronously and prevents duplicate saves", async () => {
    const pendingSave = deferred<void>();
    const saveConfirmedRegion = vi.fn().mockImplementation(() => pendingSave.promise);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({
      saveConfirmedRegion,
      savedRegionsForAsset: vi.fn().mockResolvedValue([]),
    }));

    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 20, 20, { buttons: 0 });
    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).not.toBeNull());

    const form = document.querySelector<HTMLFormElement>("[data-label-form]") as HTMLFormElement;
    const submitEvent = () => form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    submitEvent();
    submitEvent();

    expect(saveConfirmedRegion).toHaveBeenCalledOnce();
    await vi.waitFor(() => {
      expect(document.querySelector<HTMLButtonElement>('[data-label-form] button[type="submit"]')?.disabled).toBe(true);
      expect(document.body.textContent).toContain("正在保存…");
      expect(Array.from(document.querySelectorAll<HTMLButtonElement>('button[data-action="cancel-draft"]')).every((button) => button.disabled)).toBe(true);
    });
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(document.querySelector('[data-label-dialog]')).not.toBeNull();

    pendingSave.resolve(undefined);
    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).toBeNull());
    expect(saveConfirmedRegion).toHaveBeenCalledOnce();
  });

  it("locks a replacement submit synchronously and creates one replacement audit", async () => {
    const pendingReplacement = deferred<void>();
    const replaceRegion = vi.fn().mockImplementation(() => pendingReplacement.promise);
    const saveConfirmedRegion = vi.fn().mockResolvedValue(undefined);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ replaceRegion, saveConfirmedRegion }));

    document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.click();
    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 20, 20, { buttons: 0 });
    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).not.toBeNull());

    const form = document.querySelector<HTMLFormElement>("[data-label-form]") as HTMLFormElement;
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    expect(replaceRegion).toHaveBeenCalledOnce();
    expect(saveConfirmedRegion).not.toHaveBeenCalled();
    expect(document.querySelector<HTMLButtonElement>('[data-label-form] button[type="submit"]')?.disabled).toBe(true);

    pendingReplacement.resolve(undefined);
    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).toBeNull());
    expect(replaceRegion).toHaveBeenCalledOnce();
  });

  it("ignores a second left click while a free lasso is active instead of resetting its start", async () => {
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ savedRegionsForAsset: vi.fn().mockResolvedValue([]) }));

    const startCanvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(startCanvas, "pointerdown", 20, 20);

    const drawingCanvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(drawingCanvas, "pointerdown", 70, 70);
    dispatchPointer(drawingCanvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(drawingCanvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(drawingCanvas, "pointermove", 20, 20, { buttons: 0 });

    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).not.toBeNull());
  });

  it("discards an unfinished free lasso with Escape without saving", async () => {
    const saveConfirmedRegion = vi.fn().mockResolvedValue(undefined);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ saveConfirmedRegion, savedRegionsForAsset: vi.fn().mockResolvedValue([]) }));

    const canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    await vi.waitFor(() => {
      expect(document.querySelector('[data-label-dialog]')).toBeNull();
      expect(document.body.textContent).toContain("已丢弃未完成的圈选。");
    });
    expect(saveConfirmedRegion).not.toHaveBeenCalled();
    expect(document.querySelector<HTMLButtonElement>('[data-action="fit"]')?.disabled).toBe(false);
  });

  it("cancels replacement preparation with Escape or right click before drawing starts", async () => {
    const saveConfirmedRegion = vi.fn().mockResolvedValue(undefined);
    const replaceRegion = vi.fn().mockResolvedValue(undefined);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ saveConfirmedRegion, replaceRegion }));

    document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.click();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(document.body.textContent).toContain("已取消重画替换。");

    document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.click();
    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20, { button: 2, buttons: 2 });
    expect(document.body.textContent).toContain("已取消重画替换。");
    expect(document.body.textContent).toContain("1 个区域");

    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 20, 20, { buttons: 0 });
    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).not.toBeNull());
    document.querySelector<HTMLFormElement>("[data-label-form]")?.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => expect(saveConfirmedRegion).toHaveBeenCalledOnce());
    expect(replaceRegion).not.toHaveBeenCalled();
  });

  it("keeps project and media context locked while a replacement is waiting to be drawn", async () => {
    const completeCaseAndBackup = vi.fn().mockResolvedValue({
      case_id: "CASE-SYNTHETIC",
      completed_at: 0,
      capture_count: 0,
    });
    const chooseAndScanRoot = vi.fn();
    const representativeAssetIds = vi.fn().mockResolvedValue(["ASSET-SYNTHETIC"]);
    const readImageAsset = vi.fn().mockResolvedValue({
      asset_id: "ASSET-SYNTHETIC",
      mime_type: "image/png",
      data_base64: "synthetic",
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi(
      { completeCaseAndBackup, chooseAndScanRoot, representativeAssetIds, readImageAsset },
      [
        { asset_id: "ASSET-SYNTHETIC", media_kind: "image", size_bytes: 1024 },
        { asset_id: "ASSET-SECOND", media_kind: "image", size_bytes: 1024 },
      ],
    ));

    document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.click();

    await vi.waitFor(() => {
      expect(document.querySelector<HTMLSelectElement>('[data-action="campus"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="next"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="complete-backup"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="reload-project"]')?.disabled).toBe(true);
    });

    document.querySelector<HTMLSelectElement>('[data-action="campus"]')?.dispatchEvent(new Event("change", { bubbles: true }));
    document.querySelector<HTMLButtonElement>('[data-action="next"]')?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    document.querySelector<HTMLButtonElement>('[data-action="complete-backup"]')?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    document.querySelector<HTMLButtonElement>('[data-action="choose-root"]')?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));

    expect(document.body.textContent).toContain("1 / 2");
    expect(representativeAssetIds).toHaveBeenCalledOnce();
    expect(readImageAsset).toHaveBeenCalledOnce();
    expect(completeCaseAndBackup).not.toHaveBeenCalled();
    expect(chooseAndScanRoot).not.toHaveBeenCalled();
  });

  it("explains that a failed backup does not discard regions already saved with OK", async () => {
    const completeCaseAndBackup = vi.fn().mockRejectedValue({ code: "FILESYSTEM_ERROR" });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ completeCaseAndBackup }));

    document.querySelector<HTMLButtonElement>('[data-action="complete-backup"]')?.click();

    await vi.waitFor(() => expect(completeCaseAndBackup).toHaveBeenCalledWith("CASE-SYNTHETIC"));
    expect(document.body.textContent).toContain("备份文件写入失败");
    expect(document.body.textContent).toContain("已点击 OK 保存的标注仍保留在本地数据库中");
  });

  it("shows only the allowlisted backup diagnostic without leaking rejection details", async () => {
    const sensitiveMessage = "SENSITIVE_MESSAGE_SENTINEL";
    const sensitivePath = "SENSITIVE_PATH_SENTINEL";
    const sensitiveExtra = "SENSITIVE_EXTRA_SENTINEL";
    const completeCaseAndBackup = vi.fn().mockRejectedValue({
      code: "STATE_UNAVAILABLE",
      backup_stage: "VERIFY_DATABASE_OPEN",
      os_error: 5,
      message: sensitiveMessage,
      path: sensitivePath,
      extra: sensitiveExtra,
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ completeCaseAndBackup }));

    document.querySelector<HTMLButtonElement>('[data-action="complete-backup"]')?.click();

    await vi.waitFor(() => {
      expect(document.querySelector('[role="status"]')?.textContent).toBe(
        "BACKUP/VERIFY_DATABASE_OPEN/STATE_UNAVAILABLE",
      );
    });
    expect(document.body.textContent).not.toContain(sensitiveMessage);
    expect(document.body.textContent).not.toContain(sensitivePath);
    expect(document.body.textContent).not.toContain(sensitiveExtra);
  });

  it("cancels an unfinished replacement lasso with a right click without leaking its target", async () => {
    const saveConfirmedRegion = vi.fn().mockResolvedValue(undefined);
    const replaceRegion = vi.fn().mockResolvedValue(undefined);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ saveConfirmedRegion, replaceRegion }));

    const readyCanvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    installPointerCaptureMocks(readyCanvas);
    const idleContextMenu = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    readyCanvas.dispatchEvent(idleContextMenu);
    expect(idleContextMenu.defaultPrevented).toBe(true);
    dispatchPointer(readyCanvas, "pointerdown", 20, 20, { button: 2, buttons: 2 });
    expect(readyCanvas.setPointerCapture).not.toHaveBeenCalled();
    expect(document.querySelector('[data-label-dialog]')).toBeNull();
    expect(document.querySelector<HTMLButtonElement>('[data-action="pan"]')?.getAttribute("aria-pressed")).toBe("false");
    expect(document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')).not.toBeNull();

    document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.click();
    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    dispatchPointer(canvas, "pointerdown", 20, 20, { button: 2, buttons: 2 });

    await vi.waitFor(() => {
      expect(document.querySelector('[data-label-dialog]')).toBeNull();
      expect(document.body.textContent).toContain("已丢弃未完成的圈选。");
      expect(document.querySelector<HTMLButtonElement>('[data-action="fit"]')?.disabled).toBe(false);
    });
    expect(saveConfirmedRegion).not.toHaveBeenCalled();
    expect(replaceRegion).not.toHaveBeenCalled();

    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 20, 20, { buttons: 0 });

    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).not.toBeNull());
    document.querySelector<HTMLFormElement>("[data-label-form]")?.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(saveConfirmedRegion).toHaveBeenCalledOnce());
    expect(replaceRegion).not.toHaveBeenCalled();
  });

  it("suppresses the context menu and discards a label-dialog draft on right click", async () => {
    const saveConfirmedRegion = vi.fn().mockResolvedValue(undefined);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ saveConfirmedRegion, savedRegionsForAsset: vi.fn().mockResolvedValue([]) }));

    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 20, 20, { buttons: 0 });
    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).not.toBeNull());

    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    const contextMenu = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    canvas.dispatchEvent(contextMenu);
    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).toBeNull());
    expect(contextMenu.defaultPrevented).toBe(true);
    expect(saveConfirmedRegion).not.toHaveBeenCalled();
  });

  it("locks replacement during drawing and clears its target when Escape cancels the lasso", async () => {
    const saveConfirmedRegion = vi.fn().mockResolvedValue(undefined);
    const replaceRegion = vi.fn().mockResolvedValue(undefined);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    await openSyntheticAnnotation(loadedSyntheticAnnotationApi({ saveConfirmedRegion, replaceRegion }));

    document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.click();
    let canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    expect(document.querySelector<HTMLButtonElement>('[data-action="replace-region"]')?.disabled).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointerdown", 20, 20);
    canvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(canvas, "pointermove", 80, 20, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 80, 80, { buttons: 0 });
    dispatchPointer(canvas, "pointermove", 20, 20, { buttons: 0 });

    await vi.waitFor(() => expect(document.querySelector('[data-label-dialog]')).not.toBeNull());
    document.querySelector<HTMLFormElement>("[data-label-form]")?.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => expect(saveConfirmedRegion).toHaveBeenCalledOnce());
    expect(replaceRegion).not.toHaveBeenCalled();
  });

  it("locks navigation, backup, and wheel zoom while a free lasso is in progress", async () => {
    const completeCaseAndBackup = vi.fn().mockResolvedValue({
      case_id: "CASE-SYNTHETIC",
      completed_at: 0,
      capture_count: 0,
    });
    const canvasContext = installSyntheticMediaDom();
    window.__GWP_API__ = loadedSyntheticAnnotationApi(
      { completeCaseAndBackup, savedRegionsForAsset: vi.fn().mockResolvedValue([]) },
      [
        { asset_id: "ASSET-SYNTHETIC", media_kind: "image", size_bytes: 1024 },
        { asset_id: "ASSET-SECOND", media_kind: "image", size_bytes: 1024 },
      ],
    ) as never;
    boot();
    await vi.waitFor(() => expect(document.querySelector('[data-drawing-canvas]')).not.toBeNull());

    const startCanvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    dispatchPointer(startCanvas, "pointerdown", 20, 20);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("正在圈选");
      expect(document.querySelector<HTMLButtonElement>('[data-action="next"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="complete-backup"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="fit"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="reload-project"]')?.disabled).toBe(true);
      expect(document.querySelector<HTMLButtonElement>('[data-action="reload-regions"]')?.disabled).toBe(true);
    });
    const drawCallsBeforeWheel = canvasContext.drawImage.mock.calls.length;
    const drawingCanvas = document.querySelector<HTMLCanvasElement>("[data-drawing-canvas]") as HTMLCanvasElement;
    const wheel = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: -120, clientX: 50, clientY: 50 });
    drawingCanvas.dispatchEvent(wheel);
    document.querySelector<HTMLButtonElement>('[data-action="next"]')?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    document.querySelector<HTMLButtonElement>('[data-action="complete-backup"]')?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));

    expect(wheel.defaultPrevented).toBe(true);
    expect(canvasContext.drawImage).toHaveBeenCalledTimes(drawCallsBeforeWheel);
    expect(completeCaseAndBackup).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("1 / 2");
  });

  it("restores saved regions on cold startup, exposes reload, and renders blue contours", async () => {
    const savedRegionsForAsset = vi.fn().mockResolvedValue([syntheticRegion]);
    const canvasContext = installSyntheticMediaDom();
    window.__GWP_API__ = loadedSyntheticAnnotationApi({ savedRegionsForAsset }) as never;
    boot();

    await vi.waitFor(() => {
      expect(savedRegionsForAsset).toHaveBeenCalledWith("CASE-SYNTHETIC", "ASSET-SYNTHETIC");
      expect(document.body.textContent).toContain("1 个区域");
      expect(document.querySelector(".notice")?.textContent).toContain("当前图片已加载 1 个已保存区域。");
      expect(canvasContext.strokeStyles).toContain("#2563eb");
      expect(canvasContext.fillStyles).toContain("rgba(59, 130, 246, 0.12)");
    });
    expect(canvasContext.strokeStyles).not.toEqual(expect.arrayContaining(["#b45309", "#0e7490", "#c2410c"]));
    expect(canvasContext.fillStyles).not.toEqual(expect.arrayContaining(["rgba(249, 115, 22, 0.12)"]));

    savedRegionsForAsset.mockClear();
    document.querySelector<HTMLButtonElement>('[data-action="reload-regions"]')?.click();
    await vi.waitFor(() => {
      expect(savedRegionsForAsset).toHaveBeenCalledOnce();
      expect(savedRegionsForAsset).toHaveBeenCalledWith("CASE-SYNTHETIC", "ASSET-SYNTHETIC");
      expect(document.querySelector(".notice")?.textContent).toContain("已从本地数据库重新加载 1 个已保存区域。");
    });
  });
});
