// Full-screen error overlay: WebGPU unavailable, device lost, or the game server unreachable.

import type { GpuFailureReason } from "../renderer/gpu.ts";

export type ErrorKind = GpuFailureReason | "lost" | "network" | "match-over" | "other";

const HELP: Record<ErrorKind, string> = {
  "no-webgpu":
    "Loose Screws needs WebGPU. Use a current Chrome or Edge (113+), Firefox 141+ on Windows, " +
    "or Safari 26+. On Linux Chrome, enable chrome://flags/#enable-unsafe-webgpu. " +
    "WebGPU also requires a secure context (https:// or http://localhost).",
  "no-adapter":
    "Your browser supports WebGPU but found no usable GPU. Update your graphics driver, make sure " +
    "hardware acceleration is enabled in the browser settings, and check chrome://gpu for blocklisting.",
  "no-device": "The GPU refused to create a device. Close other GPU-heavy tabs or apps and reload.",
  "no-context": "The page could not attach WebGPU to its canvas. Reload the page; if that fails, try another browser.",
  lost: "The GPU device was lost (driver reset or the system reclaimed it). Reload the page to continue.",
  network:
    "Check that ls-server is running and reachable (the page connects to ws://<this host>/ws; add " +
    "?server=host:port to use another server). Reload to return to the start screen, where Practice " +
    "offline works without a server.",
  "match-over": "The server started the next match on the same level, but rejoining failed. Reload to join it.",
  other: "Something went wrong while starting the game. Reload the page to try again.",
};

export function showErrorScreen(
  parent: HTMLElement,
  title: string,
  detail: string,
  kind: ErrorKind = "other",
): void {
  parent.querySelector(".error-screen")?.remove();
  const root = document.createElement("div");
  root.className = "screen error-screen";
  const card = document.createElement("div");
  card.className = "error-card";
  const h = document.createElement("h1");
  h.textContent = title;
  const d = document.createElement("p");
  d.className = "error-detail";
  d.textContent = detail;
  const help = document.createElement("p");
  help.textContent = HELP[kind];
  const reload = document.createElement("button");
  reload.textContent = "Reload";
  reload.addEventListener("click", () => location.reload());
  card.append(h, d, help, reload);
  root.appendChild(card);
  parent.appendChild(root);
  if (document.pointerLockElement) document.exitPointerLock();
}
