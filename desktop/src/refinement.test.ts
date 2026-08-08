import { describe, expect, it } from "vitest";

import {
  BRUSH_DIAMETERS,
  pointInPolygonNorm,
  refinePolygonWithStroke,
  type NormalizedPoint,
} from "./refinement";

const IMAGE_WIDTH = 100;
const IMAGE_HEIGHT = 100;

function rectangle(left: number, top: number, right: number, bottom: number): NormalizedPoint[] {
  return [
    { x: left, y: top },
    { x: right, y: top },
    { x: right, y: bottom },
    { x: left, y: bottom },
  ];
}

function area(polygon: readonly NormalizedPoint[]): number {
  let twiceArea = 0;
  for (let index = 0; index < polygon.length; index += 1) {
    const next = polygon[(index + 1) % polygon.length];
    twiceArea += polygon[index].x * next.y - next.x * polygon[index].y;
  }
  return Math.abs(twiceArea) / 2;
}

function cross(a: NormalizedPoint, b: NormalizedPoint, c: NormalizedPoint): number {
  return (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
}

function segmentIntersects(a: NormalizedPoint, b: NormalizedPoint, c: NormalizedPoint, d: NormalizedPoint): boolean {
  const abC = cross(a, b, c);
  const abD = cross(a, b, d);
  const cdA = cross(c, d, a);
  const cdB = cross(c, d, b);
  const sign = (value: number): -1 | 0 | 1 => (value > 0 ? 1 : value < 0 ? -1 : 0);
  const pointOnSegment = (point: NormalizedPoint, start: NormalizedPoint, end: NormalizedPoint): boolean =>
    point.x >= Math.min(start.x, end.x)
    && point.x <= Math.max(start.x, end.x)
    && point.y >= Math.min(start.y, end.y)
    && point.y <= Math.max(start.y, end.y);
  const abCSign = sign(abC);
  const abDSign = sign(abD);
  const cdASign = sign(cdA);
  const cdBSign = sign(cdB);
  if (abCSign !== abDSign && cdASign !== cdBSign && abCSign !== 0 && abDSign !== 0 && cdASign !== 0 && cdBSign !== 0) {
    return true;
  }
  return (
    (abCSign === 0 && pointOnSegment(c, a, b))
    || (abDSign === 0 && pointOnSegment(d, a, b))
    || (cdASign === 0 && pointOnSegment(a, c, d))
    || (cdBSign === 0 && pointOnSegment(b, c, d))
  );
}

function isSimpleRing(polygon: readonly NormalizedPoint[]): boolean {
  for (let first = 0; first < polygon.length; first += 1) {
    for (let second = first + 1; second < polygon.length; second += 1) {
      const adjacent =
        first === second ||
        (first + 1) % polygon.length === second ||
        (second + 1) % polygon.length === first;
      if (adjacent) continue;
      if (
        segmentIntersects(
          polygon[first],
          polygon[(first + 1) % polygon.length],
          polygon[second],
          polygon[(second + 1) % polygon.length],
        )
      ) {
        return false;
      }
    }
  }
  return true;
}

function expectSuccess(result: ReturnType<typeof refinePolygonWithStroke>) {
  expect(result.ok).toBe(true);
  if (!result.ok) throw new Error(`Expected success, got ${result.reason}`);
  return result;
}

describe("source-pixel polygon refinement", () => {
  it("selects subtract from an interior start and returns a subset-style boundary", () => {
    const source = rectangle(0.2, 0.2, 0.8, 0.8);
    const result = expectSuccess(
      refinePolygonWithStroke(source, IMAGE_WIDTH, IMAGE_HEIGHT, [{ x: 0.25, y: 0.5 }, { x: 0.1, y: 0.5 }], 8),
    );

    expect(result.mode).toBe("subtract");
    expect(area(result.polygon)).toBeLessThan(area(source));
    for (const point of result.polygon) {
      expect(point.x).toBeGreaterThanOrEqual(0.2);
      expect(point.x).toBeLessThanOrEqual(0.8);
      expect(point.y).toBeGreaterThanOrEqual(0.2);
      expect(point.y).toBeLessThanOrEqual(0.8);
    }
  });

  it("selects add from an exterior start when the stroke attaches at the edge", () => {
    const result = expectSuccess(
      refinePolygonWithStroke(
        rectangle(0.3, 0.3, 0.7, 0.7),
        IMAGE_WIDTH,
        IMAGE_HEIGHT,
        [{ x: 0.74, y: 0.5 }, { x: 0.67, y: 0.5 }],
        8,
      ),
    );

    expect(result.mode).toBe("add");
    expect(Math.max(...result.polygon.map((point) => point.x))).toBeGreaterThan(0.7);
  });

  it("rejects an exterior stroke that is detached from the source region", () => {
    expect(
      refinePolygonWithStroke(
        rectangle(0.3, 0.3, 0.7, 0.7),
        IMAGE_WIDTH,
        IMAGE_HEIGHT,
        [{ x: 0.05, y: 0.05 }],
        4,
      ),
    ).toEqual({ ok: false, reason: "detached", mode: "add" });
  });

  it("rejects a subtract stroke that erases the entire source", () => {
    expect(
      refinePolygonWithStroke(rectangle(0.45, 0.45, 0.55, 0.55), IMAGE_WIDTH, IMAGE_HEIGHT, [{ x: 0.5, y: 0.5 }], 24),
    ).toEqual({ ok: false, reason: "empty", mode: "subtract" });
  });

  it("rejects a result below the backend minimum polygon area", () => {
    const width = 3840;
    const height = 2160;
    const source = rectangle(100 / width, 100 / height, 105 / width, 101 / height);

    expect(
      refinePolygonWithStroke(
        source,
        width,
        height,
        [{ x: 100 / width, y: 100.5 / height }],
        4,
      ),
    ).toEqual({ ok: false, reason: "too_small", mode: "subtract" });
  });

  it("rejects a subtract stroke that breaks the region into components", () => {
    expect(
      refinePolygonWithStroke(
        rectangle(0.2, 0.3, 0.8, 0.7),
        IMAGE_WIDTH,
        IMAGE_HEIGHT,
        [{ x: 0.5, y: 0.5 }, { x: 0.5, y: 0.2 }, { x: 0.5, y: 0.8 }],
        8,
      ),
    ).toEqual({ ok: false, reason: "disconnected", mode: "subtract" });
  });

  it("rejects an interior subtract stroke that would create a hole", () => {
    expect(
      refinePolygonWithStroke(rectangle(0.2, 0.2, 0.8, 0.8), IMAGE_WIDTH, IMAGE_HEIGHT, [{ x: 0.5, y: 0.5 }], 16),
    ).toEqual({ ok: false, reason: "hole", mode: "subtract" });
  });

  it("makes larger supported subtract brushes monotonically remove more area", () => {
    const source = rectangle(0.2, 0.2, 0.8, 0.8);
    const stroke = [{ x: 0.25, y: 0.5 }, { x: 0.1, y: 0.5 }];
    const narrow = expectSuccess(refinePolygonWithStroke(source, IMAGE_WIDTH, IMAGE_HEIGHT, stroke, 4));
    const wide = expectSuccess(refinePolygonWithStroke(source, IMAGE_WIDTH, IMAGE_HEIGHT, stroke, 16));

    expect(narrow.mode).toBe("subtract");
    expect(wide.mode).toBe("subtract");
    expect(area(wide.polygon)).toBeLessThan(area(narrow.polygon));
  });

  it("uses only source pixels: equivalent normalized expressions produce the same result", () => {
    const source = rectangle(0.2, 0.2, 0.8, 0.8);
    const sourcePixelStroke = [
      { x: 25, y: 50 },
      { x: 10, y: 50 },
    ];
    const fromPixelCoordinates = sourcePixelStroke.map(({ x, y }) => ({ x: x / IMAGE_WIDTH, y: y / IMAGE_HEIGHT }));
    const fromEquivalentNormalizedValues = [{ x: 0.25, y: 0.5 }, { x: 0.1, y: 0.5 }];

    expect(
      refinePolygonWithStroke(source, IMAGE_WIDTH, IMAGE_HEIGHT, fromPixelCoordinates, 8),
    ).toEqual(refinePolygonWithStroke(source, IMAGE_WIDTH, IMAGE_HEIGHT, fromEquivalentNormalizedValues, 8));
  });

  it("returns a finite, normalized, non-closed simple output contract", () => {
    const result = expectSuccess(
      refinePolygonWithStroke(
        rectangle(0.3, 0.3, 0.7, 0.7),
        IMAGE_WIDTH,
        IMAGE_HEIGHT,
        [{ x: 0.74, y: 0.5 }, { x: 0.67, y: 0.5 }],
        8,
      ),
    );

    expect(BRUSH_DIAMETERS).toEqual([4, 8, 16, 24]);
    expect(result.polygon.length).toBeGreaterThanOrEqual(3);
    expect(result.polygon[0]).not.toEqual(result.polygon[result.polygon.length - 1]);
    expect(new Set(result.polygon.map((point) => `${point.x}:${point.y}`)).size).toBe(result.polygon.length);
    expect(area(result.polygon)).toBeGreaterThan(0);
    expect(isSimpleRing(result.polygon)).toBe(true);
    for (const point of result.polygon) {
      expect(Number.isFinite(point.x)).toBe(true);
      expect(Number.isFinite(point.y)).toBe(true);
      expect(point.x).toBeGreaterThanOrEqual(0);
      expect(point.x).toBeLessThanOrEqual(1);
      expect(point.y).toBeGreaterThanOrEqual(0);
      expect(point.y).toBeLessThanOrEqual(1);
    }
    expect(pointInPolygonNorm({ x: 0.5, y: 0.5 }, rectangle(0.3, 0.3, 0.7, 0.7))).toBe(true);
    expect(pointInPolygonNorm({ x: 0.2, y: 0.5 }, rectangle(0.3, 0.3, 0.7, 0.7))).toBe(false);
  });
});
