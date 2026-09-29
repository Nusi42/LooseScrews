// CPU-side mesh layout shared with ls-worldgen / ls-wasm (plan 4.2 and 4.6):
// positions and normals are xyz triples, indices form triangles, and `team`
// holds one paint team id per vertex: the 0-based simulation team, NO_TEAM (255)
// = unpainted, the same ids ls-worldgen and ls-sim use. Chunk meshes arrive in
// world space, so no per-chunk transform is needed.
//
// ls-wasm maps an ls-worldgen `Mesh` to MeshData by flattening only: positions
// ([f32;3] per vertex) -> Float32Array, normals -> Float32Array, indices ->
// Uint32Array, worldgen's `teams` -> `team` (Uint8Array, values unchanged). Faces
// are counter-clockwise seen from open space (TERRAIN_CULL_MODE "back", frontFace
// "ccw"). See docs/architecture/contracts.md.

import type { Aabb } from "./frustum.ts";

/** Paint id of an unpainted vertex (ls-worldgen `NO_TEAM`). */
export const NO_TEAM = 255;

export interface MeshData {
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  team: Uint8Array;
}

export class MeshError extends Error {
  override name = "MeshError";
}

/** Checks that array lengths agree and every index is in range. */
export function validateMesh(mesh: MeshData): void {
  const { positions, normals, indices, team } = mesh;
  if (positions.length % 3 !== 0) throw new MeshError("positions length is not a multiple of 3");
  const vertexCount = positions.length / 3;
  if (normals.length !== positions.length) throw new MeshError("normals length differs from positions");
  if (team.length !== vertexCount) throw new MeshError("team must have one entry per vertex");
  if (indices.length % 3 !== 0) throw new MeshError("indices length is not a multiple of 3");
  for (let i = 0; i < indices.length; i++) {
    if ((indices[i] ?? 0) >= vertexCount) throw new MeshError(`index ${i} out of range`);
  }
}

/** Bounding box of the vertex positions; null for an empty mesh. */
export function computeAabb(positions: Float32Array): Aabb | null {
  if (positions.length < 3) return null;
  const min = { x: Infinity, y: Infinity, z: Infinity };
  const max = { x: -Infinity, y: -Infinity, z: -Infinity };
  for (let i = 0; i + 2 < positions.length; i += 3) {
    const x = positions[i] ?? 0, y = positions[i + 1] ?? 0, z = positions[i + 2] ?? 0;
    if (x < min.x) min.x = x;
    if (y < min.y) min.y = y;
    if (z < min.z) min.z = z;
    if (x > max.x) max.x = x;
    if (y > max.y) max.y = y;
    if (z > max.z) max.z = z;
  }
  return { min, max };
}

/**
 * Widens the per-vertex team bytes to u32 for the GPU. WebGPU vertex buffers
 * need 4-byte strides and the single-byte `uint8` vertex format is not yet
 * universally supported, so one u32 per vertex is the portable choice.
 */
export function expandTeams(team: Uint8Array): Uint32Array {
  const out = new Uint32Array(team.length);
  out.set(team);
  return out;
}

/** Incrementally builds a MeshData; used by procedural test geometry and primitives. */
export class MeshBuilder {
  private positions: number[] = [];
  private normals: number[] = [];
  private indices: number[] = [];
  private teams: number[] = [];

  get vertexCount(): number {
    return this.teams.length;
  }

  vertex(px: number, py: number, pz: number, nx: number, ny: number, nz: number, team = NO_TEAM): number {
    this.positions.push(px, py, pz);
    this.normals.push(nx, ny, nz);
    this.teams.push(team);
    return this.teams.length - 1;
  }

  triangle(a: number, b: number, c: number): void {
    this.indices.push(a, b, c);
  }

  /** Quad a-b-c-d in counter-clockwise order (front face). */
  quad(a: number, b: number, c: number, d: number): void {
    this.indices.push(a, b, c, a, c, d);
  }

  build(): MeshData {
    return {
      positions: new Float32Array(this.positions),
      normals: new Float32Array(this.normals),
      indices: new Uint32Array(this.indices),
      team: new Uint8Array(this.teams),
    };
  }
}

/** Uploaded mesh: separate vertex buffers per attribute plus a u32 index buffer. */
export class GpuMesh {
  readonly positionBuffer: GPUBuffer;
  readonly normalBuffer: GPUBuffer;
  readonly teamBuffer: GPUBuffer;
  readonly indexBuffer: GPUBuffer;
  readonly indexCount: number;
  readonly aabb: Aabb | null;

  constructor(device: GPUDevice, mesh: MeshData, label = "mesh") {
    validateMesh(mesh);
    this.indexCount = mesh.indices.length;
    this.aabb = computeAabb(mesh.positions);
    this.positionBuffer = createBuffer(device, mesh.positions, GPUBufferUsage.VERTEX, `${label}.positions`);
    this.normalBuffer = createBuffer(device, mesh.normals, GPUBufferUsage.VERTEX, `${label}.normals`);
    this.teamBuffer = createBuffer(device, expandTeams(mesh.team), GPUBufferUsage.VERTEX, `${label}.team`);
    this.indexBuffer = createBuffer(device, mesh.indices, GPUBufferUsage.INDEX, `${label}.indices`);
  }

  destroy(): void {
    this.positionBuffer.destroy();
    this.normalBuffer.destroy();
    this.teamBuffer.destroy();
    this.indexBuffer.destroy();
  }
}

/** Creates a buffer initialised with `data` (size padded to 4 bytes, min 4). */
export function createBuffer(
  device: GPUDevice,
  data: Float32Array | Uint32Array,
  usage: GPUBufferUsageFlags,
  label: string,
): GPUBuffer {
  const size = Math.max(4, Math.ceil(data.byteLength / 4) * 4);
  const buffer = device.createBuffer({ label, size, usage: usage | GPUBufferUsage.COPY_DST, mappedAtCreation: true });
  const Ctor = data instanceof Float32Array ? Float32Array : Uint32Array;
  new Ctor(buffer.getMappedRange()).set(data);
  buffer.unmap();
  return buffer;
}
