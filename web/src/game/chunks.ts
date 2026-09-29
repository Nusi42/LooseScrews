// Streams terrain chunk meshes from ls-wasm into the renderer with a per-frame
// time budget. ls-wasm decides the order (dirty chunks first, then missing ones
// nearest to the camera); this module only spends the budget and converts
// meshes into MeshData (contracts.md, "Chunk meshes").

import type { Vec3 } from "../math/vec3.ts";
import type { MeshData } from "../renderer/mesh.ts";
import type { ChunkMeshHandle, GameHandle } from "../wasm/api.ts";

/** Where meshes go (the renderer's `setChunk`); `null` removes a chunk. */
export type ChunkSink = (key: string, mesh: MeshData | null) => void;

export interface ChunkBudget {
  /** Milliseconds of meshing per frame (at least one chunk is always meshed). */
  millis: number;
  /** Hard cap on chunks per frame. */
  maxChunks: number;
}

/** In play: stay well inside a 16 ms frame. Loading: mesh as fast as possible. */
export const PLAY_BUDGET: ChunkBudget = { millis: 4, maxChunks: 8 };
export const LOADING_BUDGET: ChunkBudget = { millis: 40, maxChunks: 256 };

export interface ChunkStats {
  meshed: number;
  removed: number;
  pending: number;
  /** Chunks currently shown (non-empty meshes). */
  shown: number;
}

/** Copies a wasm chunk mesh into MeshData and frees the wasm object. */
export function takeMesh(m: ChunkMeshHandle): { key: string; mesh: MeshData } {
  try {
    return {
      key: m.key,
      mesh: { positions: m.positions(), normals: m.normals(), indices: m.indices(), team: m.teams() },
    };
  } finally {
    m.free();
  }
}

type GameChunks = Pick<GameHandle, "meshNext" | "takeUnloads" | "pendingChunks" | "resetMeshes">;

export class ChunkLoader {
  private readonly sink: ChunkSink;
  private readonly now: () => number;
  private readonly shown = new Set<string>();

  constructor(sink: ChunkSink, now: () => number = () => performance.now()) {
    this.sink = sink;
    this.now = now;
  }

  get shownCount(): number {
    return this.shown.size;
  }

  /** Meshes chunks around `cam` until the budget is spent; drops far chunks. */
  update(game: GameChunks, cam: Vec3, budget: ChunkBudget = PLAY_BUDGET): ChunkStats {
    let removed = 0;
    for (const key of game.takeUnloads(cam.x, cam.y, cam.z)) {
      if (this.shown.delete(key)) removed++;
      this.sink(key, null);
    }
    const start = this.now();
    let meshed = 0;
    while (meshed < budget.maxChunks && (meshed === 0 || this.now() - start < budget.millis)) {
      const handle = game.meshNext(cam.x, cam.y, cam.z);
      if (!handle) break;
      const { key, mesh } = takeMesh(handle);
      meshed++;
      if (mesh.indices.length > 0) {
        this.shown.add(key);
        this.sink(key, mesh);
      } else if (this.shown.delete(key)) {
        this.sink(key, null);
      }
    }
    return { meshed, removed, pending: game.pendingChunks(cam.x, cam.y, cam.z), shown: this.shown.size };
  }

  /** Removes every chunk from the sink and makes ls-wasm mesh them again. */
  reset(game: GameChunks): void {
    for (const key of this.shown) this.sink(key, null);
    this.shown.clear();
    game.resetMeshes();
  }
}
