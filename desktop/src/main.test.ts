import { beforeEach, describe, expect, it } from "vitest";

import {
  autoCloseThreshold,
  boot,
  canAnnotate,
  canAutoClose,
  clearCrosshairPointerState,
  customCrosshairMarkup,
  labelsMeetRequiredFields,
  polygonArea,
  primaryDrawingButtonIsPressed,
  shouldHandleMediaNavigation,
  shouldShowCustomCrosshair,
  type PointNorm,
  type TemplateField,
} from "./main";

const triangle: PointNorm[] = [
  { x: 0.1, y: 0.1 },
  { x: 0.8, y: 0.1 },
  { x: 0.4, y: 0.8 },
];

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

  it("renders an offline local status without a built-in medical template", () => {
    boot();

    expect(document.querySelector('[data-testid="offline-status"]')?.textContent).toBe("本地离线");
    expect(document.body.textContent).toContain("未加载。请先选择 JSON 或 label.txt 模板。");
  });
});
