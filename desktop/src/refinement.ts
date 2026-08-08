/**
 * Pixel-grid polygon refinement for source-image coordinates.
 *
 * A normalized coordinate describes a position in the continuous image box:
 * `{ x: 1, y: 1 }` is the lower-right image corner.  Raster decisions are made
 * at pixel centres, and the returned ring follows pixel-cell boundaries.  No
 * display-space measurement (CSS pixels, zoom, or DPR) participates here.
 */

export type NormalizedPoint = {
  x: number;
  y: number;
};

export type RefinementMode = "add" | "subtract";

export type RefinementFailure =
  | "empty"
  | "disconnected"
  | "hole"
  | "detached"
  | "too_small"
  | "non_manifold"
  | "invalid_input"
  | "too_complex";

export type RefinementResult =
  | { ok: true; polygon: NormalizedPoint[]; mode: RefinementMode }
  | { ok: false; reason: RefinementFailure; mode: RefinementMode | null };

/** The supported source-image-pixel brush diameters. */
export const BRUSH_DIAMETERS: readonly number[] = Object.freeze([4, 8, 16, 24]);

// These limits bound the typed-array allocations, scan conversion, and the
// deliberately capped O(n²) source-ring validation.  Eight mebi-pixels covers
// a 3840×2160 source image while leaving a finite upper bound for the worker.
const MAX_PIXELS = 8_388_608;
const MAX_INPUT_VERTICES = 2_048;
const MAX_STROKE_POINTS = 4_096;
const MAX_SCANLINE_EDGE_TESTS = 16_000_000;
const MAX_STROKE_RASTER_WORK = 24_000_000;
const MAX_BOUNDARY_EDGES = 250_000;
const MAX_OUTPUT_VERTICES = MAX_INPUT_VERTICES;
// Rust rejects polygons whose normalized doubled area is below 1e-6.
const MIN_NORMALIZED_POLYGON_AREA = 0.000_000_5;
const GEOMETRY_EPSILON = 1e-10;
const PIXEL_EPSILON = 1e-7;

type Validation<T> =
  | { ok: true; value: T }
  | { ok: false; reason: "invalid_input" | "too_complex" };

type Rasterized =
  | { ok: true; mask: Uint8Array }
  | { ok: false; reason: "invalid_input" | "too_complex" };

type GridRing =
  | { ok: true; x: number[]; y: number[] }
  | { ok: false; reason: "non_manifold" | "too_complex" };

type PixelPoint = { x: number; y: number };

function failure(reason: RefinementFailure, mode: RefinementMode | null): RefinementResult {
  return { ok: false, reason, mode };
}

function samePoint(a: NormalizedPoint, b: NormalizedPoint): boolean {
  return a.x === b.x && a.y === b.y;
}

function isNormalizedPoint(value: unknown): value is NormalizedPoint {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as { x?: unknown; y?: unknown };
  return (
    typeof candidate.x === "number" &&
    typeof candidate.y === "number" &&
    Number.isFinite(candidate.x) &&
    Number.isFinite(candidate.y) &&
    candidate.x >= 0 &&
    candidate.x <= 1 &&
    candidate.y >= 0 &&
    candidate.y <= 1
  );
}

function cross(a: NormalizedPoint, b: NormalizedPoint, c: NormalizedPoint): number {
  return (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
}

function pointOnSegment(point: NormalizedPoint, a: NormalizedPoint, b: NormalizedPoint): boolean {
  if (Math.abs(cross(a, b, point)) > GEOMETRY_EPSILON) return false;
  return (
    point.x >= Math.min(a.x, b.x) - GEOMETRY_EPSILON &&
    point.x <= Math.max(a.x, b.x) + GEOMETRY_EPSILON &&
    point.y >= Math.min(a.y, b.y) - GEOMETRY_EPSILON &&
    point.y <= Math.max(a.y, b.y) + GEOMETRY_EPSILON
  );
}

/**
 * Returns true for points on the boundary as well as strictly interior points.
 * Invalid normalized input is deliberately treated as outside rather than
 * throwing from a pointer-event path.
 */
export function pointInPolygonNorm(
  point: NormalizedPoint,
  polygon: readonly NormalizedPoint[],
): boolean {
  if (!isNormalizedPoint(point) || !Array.isArray(polygon) || polygon.length < 3) return false;

  let inside = false;
  let previous = polygon[polygon.length - 1];
  if (!isNormalizedPoint(previous)) return false;

  for (const current of polygon) {
    if (!isNormalizedPoint(current)) return false;
    if (pointOnSegment(point, previous, current)) return true;

    const crossesRay = (current.y > point.y) !== (previous.y > point.y);
    if (crossesRay) {
      const crossingX =
        previous.x + ((point.y - previous.y) * (current.x - previous.x)) / (current.y - previous.y);
      if (point.x < crossingX) inside = !inside;
    }
    previous = current;
  }

  return inside;
}

function signedArea(points: readonly NormalizedPoint[]): number {
  let twiceArea = 0;
  for (let index = 0; index < points.length; index += 1) {
    const next = points[(index + 1) % points.length];
    twiceArea += points[index].x * next.y - next.x * points[index].y;
  }
  return twiceArea / 2;
}

function orientationSign(value: number): -1 | 0 | 1 {
  if (value > GEOMETRY_EPSILON) return 1;
  if (value < -GEOMETRY_EPSILON) return -1;
  return 0;
}

function segmentsIntersect(
  a: NormalizedPoint,
  b: NormalizedPoint,
  c: NormalizedPoint,
  d: NormalizedPoint,
): boolean {
  const abC = orientationSign(cross(a, b, c));
  const abD = orientationSign(cross(a, b, d));
  const cdA = orientationSign(cross(c, d, a));
  const cdB = orientationSign(cross(c, d, b));

  if (abC !== abD && cdA !== cdB && abC !== 0 && abD !== 0 && cdA !== 0 && cdB !== 0) {
    return true;
  }
  return (
    (abC === 0 && pointOnSegment(c, a, b)) ||
    (abD === 0 && pointOnSegment(d, a, b)) ||
    (cdA === 0 && pointOnSegment(a, c, d)) ||
    (cdB === 0 && pointOnSegment(b, c, d))
  );
}

function adjacentEdges(first: number, second: number, length: number): boolean {
  return (
    first === second ||
    (first + 1) % length === second ||
    (second + 1) % length === first
  );
}

function hasOverlappingAdjacentEdges(points: readonly NormalizedPoint[]): boolean {
  for (let index = 0; index < points.length; index += 1) {
    const previous = points[(index + points.length - 1) % points.length];
    const current = points[index];
    const next = points[(index + 1) % points.length];
    if (Math.abs(cross(previous, current, next)) > GEOMETRY_EPSILON) continue;

    // A straight-through collinear vertex is harmless.  If both neighbouring
    // points lie in the same direction from the current vertex, the two edges
    // retrace a non-zero interval and cannot describe a simple ring.
    const previousVectorX = previous.x - current.x;
    const previousVectorY = previous.y - current.y;
    const nextVectorX = next.x - current.x;
    const nextVectorY = next.y - current.y;
    if (previousVectorX * nextVectorX + previousVectorY * nextVectorY > GEOMETRY_EPSILON) {
      return true;
    }
  }
  return false;
}

function canonicalizePolygon(polygon: readonly NormalizedPoint[]): Validation<NormalizedPoint[]> {
  if (!Array.isArray(polygon) || polygon.length > MAX_INPUT_VERTICES + 1) {
    return {
      ok: false,
      reason: Array.isArray(polygon) && polygon.length > MAX_INPUT_VERTICES + 1 ? "too_complex" : "invalid_input",
    };
  }

  const points: NormalizedPoint[] = [];
  for (const point of polygon) {
    if (!isNormalizedPoint(point)) return { ok: false, reason: "invalid_input" };
    if (points.length === 0 || !samePoint(points[points.length - 1], point)) {
      points.push({ x: point.x, y: point.y });
    }
  }
  if (points.length > 1 && samePoint(points[0], points[points.length - 1])) points.pop();
  if (points.length < 3) return { ok: false, reason: "invalid_input" };

  const seenVertices = new Set<string>();
  for (const point of points) {
    const key = `${point.x}:${point.y}`;
    if (seenVertices.has(key)) return { ok: false, reason: "invalid_input" };
    seenVertices.add(key);
  }
  if (Math.abs(signedArea(points)) < MIN_NORMALIZED_POLYGON_AREA || hasOverlappingAdjacentEdges(points)) {
    return { ok: false, reason: "invalid_input" };
  }

  // The vertex cap keeps this exact pairwise test bounded at roughly two
  // million checks.  It rejects crossings and non-adjacent vertex touches.
  for (let first = 0; first < points.length; first += 1) {
    const firstEnd = points[(first + 1) % points.length];
    for (let second = first + 1; second < points.length; second += 1) {
      if (adjacentEdges(first, second, points.length)) continue;
      if (segmentsIntersect(points[first], firstEnd, points[second], points[(second + 1) % points.length])) {
        return { ok: false, reason: "invalid_input" };
      }
    }
  }

  return { ok: true, value: points };
}

function canonicalizeStroke(stroke: readonly NormalizedPoint[]): Validation<NormalizedPoint[]> {
  if (!Array.isArray(stroke) || stroke.length === 0) return { ok: false, reason: "invalid_input" };
  if (stroke.length > MAX_STROKE_POINTS) return { ok: false, reason: "too_complex" };

  const points: NormalizedPoint[] = [];
  for (const point of stroke) {
    if (!isNormalizedPoint(point)) return { ok: false, reason: "invalid_input" };
    if (points.length === 0 || !samePoint(points[points.length - 1], point)) {
      points.push({ x: point.x, y: point.y });
    }
  }
  return points.length > 0 ? { ok: true, value: points } : { ok: false, reason: "invalid_input" };
}

function validateImageSize(imageWidth: number, imageHeight: number): Validation<number> {
  if (
    !Number.isSafeInteger(imageWidth) ||
    !Number.isSafeInteger(imageHeight) ||
    imageWidth <= 0 ||
    imageHeight <= 0
  ) {
    return { ok: false, reason: "invalid_input" };
  }
  if (imageWidth > Math.floor(MAX_PIXELS / imageHeight)) {
    return { ok: false, reason: "too_complex" };
  }
  return { ok: true, value: imageWidth * imageHeight };
}

function pixelCoordinate(point: NormalizedPoint, imageWidth: number, imageHeight: number): PixelPoint {
  return { x: point.x * imageWidth, y: point.y * imageHeight };
}

function rasterizePolygon(
  polygon: readonly NormalizedPoint[],
  imageWidth: number,
  imageHeight: number,
  pixelCount: number,
): Rasterized {
  if (polygon.length * imageHeight > MAX_SCANLINE_EDGE_TESTS) {
    return { ok: false, reason: "too_complex" };
  }

  const pixelPolygon = polygon.map((point) => pixelCoordinate(point, imageWidth, imageHeight));
  const mask = new Uint8Array(pixelCount);
  const intersections: number[] = [];

  for (let y = 0; y < imageHeight; y += 1) {
    const sampleY = y + 0.5;
    intersections.length = 0;

    for (let index = 0; index < pixelPolygon.length; index += 1) {
      const start = pixelPolygon[index];
      const end = pixelPolygon[(index + 1) % pixelPolygon.length];
      const crossesScanline = (start.y <= sampleY && end.y > sampleY) || (end.y <= sampleY && start.y > sampleY);
      if (!crossesScanline) continue;
      intersections.push(start.x + ((sampleY - start.y) * (end.x - start.x)) / (end.y - start.y));
    }

    if (intersections.length % 2 !== 0) return { ok: false, reason: "invalid_input" };
    intersections.sort((left, right) => left - right);
    const rowOffset = y * imageWidth;
    for (let index = 0; index < intersections.length; index += 2) {
      const left = intersections[index];
      const right = intersections[index + 1];
      if (!Number.isFinite(left) || !Number.isFinite(right) || right < left) {
        return { ok: false, reason: "invalid_input" };
      }

      const firstPixel = Math.max(0, Math.ceil(left - 0.5));
      const lastPixel = Math.min(imageWidth - 1, Math.floor(right - 0.5));
      if (firstPixel <= lastPixel) mask.fill(1, rowOffset + firstPixel, rowOffset + lastPixel + 1);
    }
  }

  return { ok: true, mask };
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

interface PixelBox {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
}

function capsuleBox(
  start: PixelPoint,
  end: PixelPoint,
  radius: number,
  imageWidth: number,
  imageHeight: number,
): PixelBox {
  return {
    minX: clamp(Math.ceil(Math.min(start.x, end.x) - radius - 0.5), 0, imageWidth - 1),
    maxX: clamp(Math.floor(Math.max(start.x, end.x) + radius - 0.5), 0, imageWidth - 1),
    minY: clamp(Math.ceil(Math.min(start.y, end.y) - radius - 0.5), 0, imageHeight - 1),
    maxY: clamp(Math.floor(Math.max(start.y, end.y) + radius - 0.5), 0, imageHeight - 1),
  };
}

function pointWithinCapsule(
  x: number,
  y: number,
  start: PixelPoint,
  end: PixelPoint,
  radiusSquared: number,
): boolean {
  const directionX = end.x - start.x;
  const directionY = end.y - start.y;
  const lengthSquared = directionX * directionX + directionY * directionY;
  let t = 0;
  if (lengthSquared > 0) {
    t = ((x - start.x) * directionX + (y - start.y) * directionY) / lengthSquared;
    t = clamp(t, 0, 1);
  }
  const closestX = start.x + t * directionX;
  const closestY = start.y + t * directionY;
  const distanceX = x - closestX;
  const distanceY = y - closestY;
  return distanceX * distanceX + distanceY * distanceY <= radiusSquared + PIXEL_EPSILON;
}

function rasterizeStroke(
  stroke: readonly NormalizedPoint[],
  imageWidth: number,
  imageHeight: number,
  pixelCount: number,
  brushDiameterPx: number,
): Rasterized {
  const radius = brushDiameterPx / 2;
  const pixelStroke = stroke.map((point) => pixelCoordinate(point, imageWidth, imageHeight));
  const segmentCount = Math.max(1, pixelStroke.length - 1);
  let estimatedWork = 0;

  for (let index = 0; index < segmentCount; index += 1) {
    const start = pixelStroke[index];
    const end = pixelStroke[Math.min(index + 1, pixelStroke.length - 1)];
    const box = capsuleBox(start, end, radius, imageWidth, imageHeight);
    estimatedWork += (box.maxX - box.minX + 1) * (box.maxY - box.minY + 1);
    if (estimatedWork > MAX_STROKE_RASTER_WORK) return { ok: false, reason: "too_complex" };
  }

  const mask = new Uint8Array(pixelCount);
  const radiusSquared = radius * radius;
  for (let index = 0; index < segmentCount; index += 1) {
    const start = pixelStroke[index];
    const end = pixelStroke[Math.min(index + 1, pixelStroke.length - 1)];
    const box = capsuleBox(start, end, radius, imageWidth, imageHeight);
    for (let y = box.minY; y <= box.maxY; y += 1) {
      const rowOffset = y * imageWidth;
      for (let x = box.minX; x <= box.maxX; x += 1) {
        if (pointWithinCapsule(x + 0.5, y + 0.5, start, end, radiusSquared)) {
          mask[rowOffset + x] = 1;
        }
      }
    }
  }

  return { ok: true, mask };
}

function hasForeground(mask: Uint8Array): boolean {
  for (let index = 0; index < mask.length; index += 1) {
    if (mask[index] !== 0) return true;
  }
  return false;
}

function isFilled(mask: Uint8Array, imageWidth: number, imageHeight: number, x: number, y: number): boolean {
  return x >= 0 && x < imageWidth && y >= 0 && y < imageHeight && mask[y * imageWidth + x] !== 0;
}

/**
 * Pixel squares that overlap, share an edge, or meet at a corner count as a
 * contact.  Corner-only contact still goes through topology validation and is
 * rejected as disconnected/non-manifold when it cannot make a simple region.
 */
function footprintTouchesRegion(
  region: Uint8Array,
  footprint: Uint8Array,
  imageWidth: number,
  imageHeight: number,
): boolean {
  for (let y = 0; y < imageHeight; y += 1) {
    const rowOffset = y * imageWidth;
    for (let x = 0; x < imageWidth; x += 1) {
      const index = rowOffset + x;
      if (footprint[index] === 0) continue;
      for (let offsetY = -1; offsetY <= 1; offsetY += 1) {
        for (let offsetX = -1; offsetX <= 1; offsetX += 1) {
          if (isFilled(region, imageWidth, imageHeight, x + offsetX, y + offsetY)) return true;
        }
      }
    }
  }
  return false;
}

function countComponents(
  mask: Uint8Array,
  imageWidth: number,
  imageHeight: number,
  visited: Uint8Array,
  queue: Int32Array,
): number {
  let components = 0;
  for (let start = 0; start < mask.length; start += 1) {
    if (mask[start] === 0 || visited[start] !== 0) continue;
    components += 1;
    if (components > 1) return components;

    let head = 0;
    let tail = 0;
    queue[tail] = start;
    tail += 1;
    visited[start] = 1;
    while (head < tail) {
      const index = queue[head];
      head += 1;
      const x = index % imageWidth;
      const y = Math.floor(index / imageWidth);
      if (x > 0) {
        const neighbour = index - 1;
        if (mask[neighbour] !== 0 && visited[neighbour] === 0) {
          visited[neighbour] = 1;
          queue[tail] = neighbour;
          tail += 1;
        }
      }
      if (x + 1 < imageWidth) {
        const neighbour = index + 1;
        if (mask[neighbour] !== 0 && visited[neighbour] === 0) {
          visited[neighbour] = 1;
          queue[tail] = neighbour;
          tail += 1;
        }
      }
      if (y > 0) {
        const neighbour = index - imageWidth;
        if (mask[neighbour] !== 0 && visited[neighbour] === 0) {
          visited[neighbour] = 1;
          queue[tail] = neighbour;
          tail += 1;
        }
      }
      if (y + 1 < imageHeight) {
        const neighbour = index + imageWidth;
        if (mask[neighbour] !== 0 && visited[neighbour] === 0) {
          visited[neighbour] = 1;
          queue[tail] = neighbour;
          tail += 1;
        }
      }
    }
  }
  return components;
}

function hasHole(
  mask: Uint8Array,
  imageWidth: number,
  imageHeight: number,
  visited: Uint8Array,
  queue: Int32Array,
): boolean {
  const enqueueBackground = (index: number, tail: number): number => {
    if (mask[index] === 0 && visited[index] === 0) {
      visited[index] = 1;
      queue[tail] = index;
      return tail + 1;
    }
    return tail;
  };

  let head = 0;
  let tail = 0;
  for (let x = 0; x < imageWidth; x += 1) {
    tail = enqueueBackground(x, tail);
    tail = enqueueBackground((imageHeight - 1) * imageWidth + x, tail);
  }
  for (let y = 1; y + 1 < imageHeight; y += 1) {
    tail = enqueueBackground(y * imageWidth, tail);
    tail = enqueueBackground(y * imageWidth + imageWidth - 1, tail);
  }

  while (head < tail) {
    const index = queue[head];
    head += 1;
    const x = index % imageWidth;
    const y = Math.floor(index / imageWidth);
    if (x > 0) {
      const neighbour = index - 1;
      if (mask[neighbour] === 0 && visited[neighbour] === 0) {
        visited[neighbour] = 1;
        queue[tail] = neighbour;
        tail += 1;
      }
    }
    if (x + 1 < imageWidth) {
      const neighbour = index + 1;
      if (mask[neighbour] === 0 && visited[neighbour] === 0) {
        visited[neighbour] = 1;
        queue[tail] = neighbour;
        tail += 1;
      }
    }
    if (y > 0) {
      const neighbour = index - imageWidth;
      if (mask[neighbour] === 0 && visited[neighbour] === 0) {
        visited[neighbour] = 1;
        queue[tail] = neighbour;
        tail += 1;
      }
    }
    if (y + 1 < imageHeight) {
      const neighbour = index + imageWidth;
      if (mask[neighbour] === 0 && visited[neighbour] === 0) {
        visited[neighbour] = 1;
        queue[tail] = neighbour;
        tail += 1;
      }
    }
  }

  for (let index = 0; index < mask.length; index += 1) {
    if (mask[index] === 0 && visited[index] === 0) return true;
  }
  return false;
}

function hasDiagonalAmbiguity(mask: Uint8Array, imageWidth: number, imageHeight: number): boolean {
  for (let y = 1; y < imageHeight; y += 1) {
    for (let x = 1; x < imageWidth; x += 1) {
      const northWest = mask[(y - 1) * imageWidth + x - 1] !== 0;
      const northEast = mask[(y - 1) * imageWidth + x] !== 0;
      const southWest = mask[y * imageWidth + x - 1] !== 0;
      const southEast = mask[y * imageWidth + x] !== 0;
      if (
        (northWest && southEast && !northEast && !southWest) ||
        (northEast && southWest && !northWest && !southEast)
      ) {
        return true;
      }
    }
  }
  return false;
}

function countBoundaryEdges(mask: Uint8Array, imageWidth: number, imageHeight: number): number | null {
  let edges = 0;
  for (let y = 0; y < imageHeight; y += 1) {
    for (let x = 0; x < imageWidth; x += 1) {
      if (!isFilled(mask, imageWidth, imageHeight, x, y)) continue;
      if (!isFilled(mask, imageWidth, imageHeight, x, y - 1)) edges += 1;
      if (!isFilled(mask, imageWidth, imageHeight, x + 1, y)) edges += 1;
      if (!isFilled(mask, imageWidth, imageHeight, x, y + 1)) edges += 1;
      if (!isFilled(mask, imageWidth, imageHeight, x - 1, y)) edges += 1;
      if (edges > MAX_BOUNDARY_EDGES) return null;
    }
  }
  return edges;
}

const DIRECTION_X = [1, 0, -1, 0] as const;
const DIRECTION_Y = [0, 1, 0, -1] as const;

function outgoingBoundaryDirections(
  mask: Uint8Array,
  imageWidth: number,
  imageHeight: number,
  vertexX: number,
  vertexY: number,
): number[] {
  const directions: number[] = [];
  // Each directed edge has foreground to its right in image coordinates.
  if (isFilled(mask, imageWidth, imageHeight, vertexX, vertexY) && !isFilled(mask, imageWidth, imageHeight, vertexX, vertexY - 1)) {
    directions.push(0); // east: top of the south-east cell
  }
  if (isFilled(mask, imageWidth, imageHeight, vertexX - 1, vertexY) && !isFilled(mask, imageWidth, imageHeight, vertexX, vertexY)) {
    directions.push(1); // south: right of the south-west cell
  }
  if (isFilled(mask, imageWidth, imageHeight, vertexX - 1, vertexY - 1) && !isFilled(mask, imageWidth, imageHeight, vertexX - 1, vertexY)) {
    directions.push(2); // west: bottom of the north-west cell
  }
  if (isFilled(mask, imageWidth, imageHeight, vertexX, vertexY - 1) && !isFilled(mask, imageWidth, imageHeight, vertexX - 1, vertexY - 1)) {
    directions.push(3); // north: left of the north-east cell
  }
  return directions;
}

function traceSingleBoundary(mask: Uint8Array, imageWidth: number, imageHeight: number): GridRing {
  const boundaryEdges = countBoundaryEdges(mask, imageWidth, imageHeight);
  if (boundaryEdges === null) return { ok: false, reason: "too_complex" };
  if (boundaryEdges === 0) return { ok: false, reason: "non_manifold" };

  let startX = -1;
  let startY = -1;
  for (let y = 0; y < imageHeight && startX < 0; y += 1) {
    for (let x = 0; x < imageWidth; x += 1) {
      if (isFilled(mask, imageWidth, imageHeight, x, y) && !isFilled(mask, imageWidth, imageHeight, x, y - 1)) {
        startX = x;
        startY = y;
        break;
      }
    }
  }
  if (startX < 0 || startY < 0) return { ok: false, reason: "non_manifold" };

  const verticesX = [startX];
  const verticesY = [startY];
  const vertexStride = imageWidth + 1;
  const seenVertices = new Set<number>([startY * vertexStride + startX]);
  let currentX = startX;
  let currentY = startY;

  for (let step = 0; step < boundaryEdges; step += 1) {
    const directions = outgoingBoundaryDirections(mask, imageWidth, imageHeight, currentX, currentY);
    if (directions.length !== 1) return { ok: false, reason: "non_manifold" };
    const direction = directions[0];
    currentX += DIRECTION_X[direction];
    currentY += DIRECTION_Y[direction];

    if (currentX === startX && currentY === startY) {
      if (step + 1 !== boundaryEdges) return { ok: false, reason: "non_manifold" };
      return { ok: true, x: verticesX, y: verticesY };
    }

    const key = currentY * vertexStride + currentX;
    if (seenVertices.has(key)) return { ok: false, reason: "non_manifold" };
    seenVertices.add(key);
    verticesX.push(currentX);
    verticesY.push(currentY);
  }

  return { ok: false, reason: "non_manifold" };
}

function simplifyGridRing(ring: GridRing & { ok: true }): GridRing & { ok: true } {
  const simplifiedX: number[] = [];
  const simplifiedY: number[] = [];
  const length = ring.x.length;
  for (let index = 0; index < length; index += 1) {
    const previous = (index + length - 1) % length;
    const next = (index + 1) % length;
    const incomingX = ring.x[index] - ring.x[previous];
    const incomingY = ring.y[index] - ring.y[previous];
    const outgoingX = ring.x[next] - ring.x[index];
    const outgoingY = ring.y[next] - ring.y[index];
    const collinear = incomingX * outgoingY - incomingY * outgoingX === 0;
    const sameDirection = incomingX * outgoingX + incomingY * outgoingY > 0;
    if (!collinear || !sameDirection) {
      simplifiedX.push(ring.x[index]);
      simplifiedY.push(ring.y[index]);
    }
  }
  return { ok: true, x: simplifiedX, y: simplifiedY };
}

function polygonFromGridRing(
  ring: GridRing & { ok: true },
  imageWidth: number,
  imageHeight: number,
): NormalizedPoint[] | null {
  if (ring.x.length < 3 || ring.x.length !== ring.y.length) return null;

  const polygon = ring.x.map((x, index) => ({ x: x / imageWidth, y: ring.y[index] / imageHeight }));
  const seenVertices = new Set<string>();
  for (const point of polygon) {
    if (!isNormalizedPoint(point)) return null;
    const key = `${point.x}:${point.y}`;
    if (seenVertices.has(key)) return null;
    seenVertices.add(key);
  }
  if (Math.abs(signedArea(polygon)) < MIN_NORMALIZED_POLYGON_AREA) return null;
  return polygon;
}

function topologyFailure(
  mask: Uint8Array,
  imageWidth: number,
  imageHeight: number,
): RefinementFailure | null {
  if (!hasForeground(mask)) return "empty";

  const visited = new Uint8Array(mask.length);
  const queue = new Int32Array(mask.length);
  if (countComponents(mask, imageWidth, imageHeight, visited, queue) !== 1) return "disconnected";
  if (hasDiagonalAmbiguity(mask, imageWidth, imageHeight)) return "non_manifold";

  visited.fill(0);
  if (hasHole(mask, imageWidth, imageHeight, visited, queue)) return "hole";

  const ring = traceSingleBoundary(mask, imageWidth, imageHeight);
  if (!ring.ok) return ring.reason;
  const simplified = simplifyGridRing(ring);
  if (simplified.x.length > MAX_OUTPUT_VERTICES) return "too_complex";
  const normalizedRing = simplified.x.map((x, index) => ({
    x: x / imageWidth,
    y: simplified.y[index] / imageHeight,
  }));
  if (Math.abs(signedArea(normalizedRing)) < MIN_NORMALIZED_POLYGON_AREA) return "too_small";
  return polygonFromGridRing(simplified, imageWidth, imageHeight) === null ? "non_manifold" : null;
}

function outputPolygon(
  mask: Uint8Array,
  imageWidth: number,
  imageHeight: number,
): { ok: true; polygon: NormalizedPoint[] } | { ok: false; reason: "non_manifold" | "too_complex" } {
  const ring = traceSingleBoundary(mask, imageWidth, imageHeight);
  if (!ring.ok) return ring;
  const simplified = simplifyGridRing(ring);
  if (simplified.x.length > MAX_OUTPUT_VERTICES) return { ok: false, reason: "too_complex" };
  const polygon = polygonFromGridRing(simplified, imageWidth, imageHeight);
  return polygon === null ? { ok: false, reason: "non_manifold" } : { ok: true, polygon };
}

/**
 * Refines a normalized simple polygon using a source-image-pixel brush stroke.
 * The operation is atomic: failures return no partially altered polygon.
 */
export function refinePolygonWithStroke(
  polygon: readonly NormalizedPoint[],
  imageWidth: number,
  imageHeight: number,
  stroke: readonly NormalizedPoint[],
  brushDiameterPx: number,
): RefinementResult {
  const image = validateImageSize(imageWidth, imageHeight);
  if (!image.ok) return failure(image.reason, null);
  if (!BRUSH_DIAMETERS.includes(brushDiameterPx)) return failure("invalid_input", null);

  const sourcePolygon = canonicalizePolygon(polygon);
  if (!sourcePolygon.ok) return failure(sourcePolygon.reason, null);
  const sourceStroke = canonicalizeStroke(stroke);
  if (!sourceStroke.ok) return failure(sourceStroke.reason, null);

  const mode: RefinementMode = pointInPolygonNorm(sourceStroke.value[0], sourcePolygon.value)
    ? "subtract"
    : "add";
  const rasterizedPolygon = rasterizePolygon(sourcePolygon.value, imageWidth, imageHeight, image.value);
  if (!rasterizedPolygon.ok) return failure(rasterizedPolygon.reason, mode);
  if (!hasForeground(rasterizedPolygon.mask)) return failure("invalid_input", mode);

  const rasterizedStroke = rasterizeStroke(
    sourceStroke.value,
    imageWidth,
    imageHeight,
    image.value,
    brushDiameterPx,
  );
  if (!rasterizedStroke.ok) return failure(rasterizedStroke.reason, mode);

  if (mode === "add" && !footprintTouchesRegion(rasterizedPolygon.mask, rasterizedStroke.mask, imageWidth, imageHeight)) {
    return failure("detached", mode);
  }

  for (let index = 0; index < rasterizedPolygon.mask.length; index += 1) {
    if (rasterizedStroke.mask[index] === 0) continue;
    rasterizedPolygon.mask[index] = mode === "add" ? 1 : 0;
  }

  const topology = topologyFailure(rasterizedPolygon.mask, imageWidth, imageHeight);
  if (topology !== null) return failure(topology, mode);
  const output = outputPolygon(rasterizedPolygon.mask, imageWidth, imageHeight);
  if (!output.ok) return failure(output.reason, mode);
  return { ok: true, polygon: output.polygon, mode };
}
