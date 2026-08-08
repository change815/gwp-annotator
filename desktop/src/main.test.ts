import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  autoCloseThreshold,
  boot,
  canAnnotate,
  canAutoClose,
  clearCrosshairPointerState,
  customCrosshairMarkup,
  decodeUtf8Template,
  formatVideoTime,
  labelsMeetRequiredFields,
  LEGACY_TEMPLATE_ENCODING_WARNING,
  polygonArea,
  primaryDrawingButtonIsPressed,
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
    ...overrides,
  };
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
    vi.restoreAllMocks();
  });

  it("renders an offline local status without a built-in medical template", () => {
    boot();

    expect(document.querySelector('[data-testid="offline-status"]')?.textContent).toBe("本地离线");
    expect(document.body.textContent).toContain("未加载。请选择 label-template-v1.toml。");
    expect(document.querySelector<HTMLInputElement>('[data-action="template-file"]')?.disabled).toBe(false);
    expect(document.querySelector<HTMLButtonElement>('[data-action="storage-location"]')).not.toBeNull();
    expect(document.querySelector<HTMLButtonElement>('[data-action="forget-position"]')?.disabled).toBe(false);
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
});
