/**
 * Ramer–Douglas–Peucker polyline simplification.
 *
 * Used on stroke commit (pointerup) to compress freehand paths before they
 * land in the CRDT — peers see uncompressed motion during the live stream
 * (every ~16 ms tail-append), but the persisted shape only carries the
 * vertices needed to reproduce the curve within `epsilon` pixels.
 *
 * Tolerance trade-off:
 *   - 0.5 px keeps near-perfect fidelity, ~50% byte savings on smooth strokes.
 *   - 1–2 px is typical for whiteboards (visually identical).
 *   - >4 px starts visibly cornering the curve.
 */
export interface Point {
  x: number;
  y: number;
}

export function simplify(points: readonly Point[], epsilon = 1): Point[] {
  if (points.length < 3) return points.slice();
  return rdp(points, 0, points.length - 1, epsilon);
}

function rdp(points: readonly Point[], first: number, last: number, eps: number): Point[] {
  let maxDist = 0;
  let index = first;
  const start = points[first];
  const end = points[last];

  for (let i = first + 1; i < last; i++) {
    const d = perpendicularDistance(points[i], start, end);
    if (d > maxDist) {
      maxDist = d;
      index = i;
    }
  }

  if (maxDist > eps) {
    const left = rdp(points, first, index, eps);
    const right = rdp(points, index, last, eps);
    // `left` ends with `points[index]` and `right` starts with the same
    // point — slice the dup off the right side so callers don't see it twice.
    return left.concat(right.slice(1));
  }
  return [start, end];
}

function perpendicularDistance(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  if (dx === 0 && dy === 0) {
    const ex = p.x - a.x;
    const ey = p.y - a.y;
    return Math.sqrt(ex * ex + ey * ey);
  }
  const num = Math.abs(dy * p.x - dx * p.y + b.x * a.y - b.y * a.x);
  const den = Math.sqrt(dx * dx + dy * dy);
  return num / den;
}
