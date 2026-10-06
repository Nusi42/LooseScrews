// Small inline SVG icons for the mode HUD (M3 C1): objective markers and
// alerts. Stroke-only, `currentColor`, 16x16 view box, so CSS colours them
// with the marker's side colour.

export type ModeIcon = "zone" | "nextZone" | "wall" | "core" | "carrier" | "extraction" | "reactor" | "colour" | "skull" | "spectate" | "exit" | "stroke" | "runner";

const svg = (body: string): string =>
  `<svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

/** SVG markup per icon. */
export const MODE_ICONS: Readonly<Record<ModeIcon, string>> = {
  // A circle with a dot: the safe zone.
  zone: svg('<circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="1.4" fill="currentColor"/>'),
  // A dashed circle with corner ticks: where the zone goes next.
  nextZone: svg('<circle cx="8" cy="8" r="4.5" stroke-dasharray="2.4 1.6"/><path d="M8 1v2M8 13v2M1 8h2M13 8h2"/>'),
  // Arrows pressing inward: the walls.
  wall: svg('<path d="M1.5 3v10M14.5 3v10M4 8h3M5.5 6.5 7 8 5.5 9.5M12 8H9M10.5 6.5 9 8l1.5 1.5"/>'),
  // A faceted crystal: the Heist core.
  core: svg('<path d="M8 1.5 12.5 6 8 14.5 3.5 6Z"/><path d="M3.5 6h9M6 6l2 8.5L10 6"/>'),
  // A crystal over a chevron: the carrier.
  carrier: svg('<path d="M8 1.5 10.5 4 8 7.5 5.5 4Z"/><path d="m3 10 5 4 5-4"/>'),
  // An arrow up out of a ring: the extraction zone.
  extraction: svg('<ellipse cx="8" cy="12" rx="6" ry="2.4"/><path d="M8 11V2M5 5l3-3 3 3"/>'),
  // A trefoil: the reactor.
  reactor: svg('<circle cx="8" cy="8" r="1.6" fill="currentColor"/><path d="M8 5.2 6 1.8a7 7 0 0 1 4 0ZM10.4 9.4l3.8.2a7 7 0 0 1-2 3.5ZM5.6 9.4l-3.8.2a7 7 0 0 0 2 3.5Z" fill="currentColor"/>'),
  // A drop: Last Color Standing's colour.
  colour: svg('<path d="M8 1.8C8 1.8 3.5 7 3.5 10a4.5 4.5 0 0 0 9 0C12.5 7 8 1.8 8 1.8Z"/>'),
  skull: svg('<path d="M3 7a5 5 0 0 1 10 0v3l-1.5 1v2.5h-7V11L3 10Z"/><circle cx="6" cy="8" r="1" fill="currentColor"/><circle cx="10" cy="8" r="1" fill="currentColor"/>'),
  spectate: svg('<path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8Z"/><circle cx="8" cy="8" r="2"/>'),
  // The Architect (M4 C4): a door with an arrow out (the exit gate)...
  exit: svg('<path d="M9.5 2.5h-6v11h6"/><path d="M7 8h7M11.5 5.5 14 8l-2.5 2.5"/>'),
  // ...a pencil (a stroke)...
  stroke: svg('<path d="M2.5 13.5 3.3 10.4 10.8 2.9a1.6 1.6 0 0 1 2.3 2.3L5.6 12.7Z"/><path d="M9.6 4.1l2.3 2.3"/>'),
  // ...and a ship's chevron (a runner, seen through rock).
  runner: svg('<path d="M8 2 13.5 13 8 10.2 2.5 13Z"/>'),
};

/** SVG markup of an icon ("" for none or unknown). */
export function iconSvg(icon: string): string {
  return (MODE_ICONS as Record<string, string>)[icon] ?? "";
}
