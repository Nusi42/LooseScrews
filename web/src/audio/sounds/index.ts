// Every built-in sound group. A package adding sounds (event visuals, race
// sounds, mood drones) adds its file here; ids are `group.name` and unique.

import type { SoundDef } from "../library.ts";
import { ANNOUNCER_SOUNDS } from "./announcer.ts";
import { IMPACT_SOUNDS } from "./impacts.ts";
import { MECHANICS_SOUNDS } from "./mechanics.ts";
import { WEAPON_SOUNDS } from "./weapons.ts";
import { WORLD_SOUNDS } from "./world.ts";

export const BUILTIN_SOUNDS: readonly SoundDef[] = [...WEAPON_SOUNDS, ...IMPACT_SOUNDS, ...MECHANICS_SOUNDS, ...ANNOUNCER_SOUNDS, ...WORLD_SOUNDS];
