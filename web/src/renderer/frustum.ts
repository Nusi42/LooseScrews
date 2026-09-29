// View-frustum culling against axis-aligned bounding boxes.

import type { Mat4 } from "../math/mat4.ts";
import type { Vec3 } from "../math/vec3.ts";

export interface Aabb {
  min: Vec3;
  max: Vec3;
}

/** Six planes (a, b, c, d) with a*x + b*y + c*z + d >= 0 inside; normals point inward. */
export class Frustum {
  readonly planes = new Float32Array(24);

  /**
   * Extracts planes from a view-projection matrix (Gribb/Hartmann) for WebGPU's
   * [0, 1] depth range: left, right, bottom, top, near (row 2), far (row 3 - row 2).
   */
  setFromViewProj(m: Mat4): void {
    const row = (r: number): [number, number, number, number] => [
      m[r] ?? 0,
      m[4 + r] ?? 0,
      m[8 + r] ?? 0,
      m[12 + r] ?? 0,
    ];
    const r0 = row(0), r1 = row(1), r2 = row(2), r3 = row(3);
    const planes: [number, number, number, number][] = [
      [r3[0] + r0[0], r3[1] + r0[1], r3[2] + r0[2], r3[3] + r0[3]],
      [r3[0] - r0[0], r3[1] - r0[1], r3[2] - r0[2], r3[3] - r0[3]],
      [r3[0] + r1[0], r3[1] + r1[1], r3[2] + r1[2], r3[3] + r1[3]],
      [r3[0] - r1[0], r3[1] - r1[1], r3[2] - r1[2], r3[3] - r1[3]],
      [r2[0], r2[1], r2[2], r2[3]],
      [r3[0] - r2[0], r3[1] - r2[1], r3[2] - r2[2], r3[3] - r2[3]],
    ];
    planes.forEach((p, i) => {
      const len = Math.hypot(p[0], p[1], p[2]) || 1;
      this.planes.set([p[0] / len, p[1] / len, p[2] / len, p[3] / len], i * 4);
    });
  }

  /** Conservative test: false only when the box is fully outside one plane. */
  intersectsAabb(box: Aabb): boolean {
    const p = this.planes;
    for (let i = 0; i < 24; i += 4) {
      const a = p[i] ?? 0, b = p[i + 1] ?? 0, c = p[i + 2] ?? 0, d = p[i + 3] ?? 0;
      // "Positive vertex": the box corner furthest along the plane normal.
      const x = a >= 0 ? box.max.x : box.min.x;
      const y = b >= 0 ? box.max.y : box.min.y;
      const z = c >= 0 ? box.max.z : box.min.z;
      if (a * x + b * y + c * z + d < 0) return false;
    }
    return true;
  }
}
