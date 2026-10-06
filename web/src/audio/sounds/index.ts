// Every built-in sound group. A package adding sounds (event visuals, race
// sounds, mood drones) adds its file here; ids are `group.name` and unique.

import type { SoundDef } from "../library.ts";
import { ANNOUNCER_SOUNDS } from "./announcer.ts";
import { EVENT_SOUNDS } from "./events.ts";
import { ARCHITECT_SOUNDS } from "./architect.ts";
import { IMPACT_SOUNDS } from "./impacts.ts";
import { MECHANICS_SOUNDS } from "./mechanics.ts";
import { WEAPON_SOUNDS } from "./weapons.ts";
import { VISITOR_SOUNDS } from "./visitors.ts";
import { WORLD_SOUNDS } from "./world.ts";
import { SPECTACLE_SOUNDS } from "./spectacles.ts";
import { RACE_SOUNDS } from "./race.ts";

export const BUILTIN_SOUNDS: readonly SoundDef[] = [...WEAPON_SOUNDS, ...IMPACT_SOUNDS, ...MECHANICS_SOUNDS, ...ANNOUNCER_SOUNDS, ...WORLD_SOUNDS, ...EVENT_SOUNDS, ...VISITOR_SOUNDS, ...SPECTACLE_SOUNDS, ...ARCHITECT_SOUNDS, ...RACE_SOUNDS];
