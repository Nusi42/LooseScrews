// Offline preview "world" for the test scene: a few drone ships flying the
// tunnel, visual-only projectiles and placeholder resource meters so the HUD
// and controls can be exercised with `npm run dev`.
//
// Everything here is throwaway until wave 3: real ship state, weapons and
// resources come from ls-sim (server snapshots + WASM prediction) and the
// numbers from data/weapons/*.ron. The constants below are preview-only
// approximations of the design doc and must not be treated as gameplay data.

import * as q from "../math/quat.ts";
import { addScaled, scale, type Vec3 } from "../math/vec3.ts";
import type { InputState } from "../input/input.ts";
import type { FlightCamera } from "../input/flightCamera.ts";
import type { Instance, Rgb } from "../renderer/renderer.ts";
import type { HudValues } from "../ui/hud.ts";
import type { TestScene } from "../scene/testScene.ts";

const PREVIEW = {
  matchSeconds: 300,
  energyMax: 100,
  energyRegen: 18, // per second, only while not boosting
  boostMax: 100,
  boostDrain: 30, // per second while boosting
  boostFromBraking: 1.5, // boost gained per u/s of speed lost
  hullMax: 100,
  scavengeHull: 4, // per second while holding R
  scavengeScrap: 6,
  laser: { interval: 0.18, energy: 4, speed: 30, lifetime: 3, color: [1.0, 0.35, 0.2] as Rgb },
  rivet: { interval: 0.09, scrap: 1, speed: 220, lifetime: 0.8, color: [0.95, 0.9, 0.7] as Rgb },
};

interface Drone {
  t: number;
  speed: number; // loop fraction per second
  lateral: number;
  vertical: number;
  team: number;
}

interface Projectile {
  pos: Vec3;
  vel: Vec3;
  rot: q.Quat;
  ttl: number;
  color: Rgb;
  scale: number;
}

/** Drone hull colour per 0-based team. */
const TEAM_HULL: Rgb[] = [
  [0.95, 0.5, 0.2],
  [0.25, 0.6, 0.95],
];

export class DemoWorld {
  private readonly scene: TestScene;
  private readonly drones: Drone[] = [];
  private projectiles: Projectile[] = [];
  private primaryCooldown = 0;
  private secondaryCooldown = 0;
  private lastSpeed = 0;
  private timeLeft = PREVIEW.matchSeconds;
  hull = PREVIEW.hullMax;
  energy = PREVIEW.energyMax;
  boost = PREVIEW.boostMax;
  scrap = 40;
  lastWeapon = "Scrap Laser";

  constructor(scene: TestScene, droneCount = 7) {
    this.scene = scene;
    for (let i = 0; i < droneCount; i++) {
      this.drones.push({
        t: i / droneCount,
        speed: (0.012 + 0.004 * (i % 3)) * (i % 2 === 0 ? 1 : -1),
        lateral: ((i * 37) % 7) - 3,
        vertical: ((i * 53) % 5) - 2,
        team: i % 2,
      });
    }
  }

  /** Whether boost is available this frame (meter not empty). */
  canBoost(input: InputState): boolean {
    return input.boost && this.boost > 0;
  }

  update(dt: number, input: InputState, camera: FlightCamera, boosting: boolean): void {
    this.timeLeft = Math.max(0, this.timeLeft - dt);

    // Resources (preview approximations of the design doc rules).
    if (boosting) {
      this.boost = Math.max(0, this.boost - PREVIEW.boostDrain * dt);
    } else {
      this.energy = Math.min(PREVIEW.energyMax, this.energy + PREVIEW.energyRegen * dt);
    }
    const speed = camera.speed;
    if (speed < this.lastSpeed) {
      this.boost = Math.min(PREVIEW.boostMax, this.boost + (this.lastSpeed - speed) * PREVIEW.boostFromBraking);
    }
    this.lastSpeed = speed;
    if (input.scavenge) {
      this.hull = Math.min(PREVIEW.hullMax, this.hull + PREVIEW.scavengeHull * dt);
      this.scrap += PREVIEW.scavengeScrap * dt;
    }

    // Weapons: primary = Scrap Laser (energy, overdrive burns hull), secondary = Rivet Gun (scrap).
    this.primaryCooldown -= dt;
    this.secondaryCooldown -= dt;
    if (input.firePrimary && this.primaryCooldown <= 0) {
      const cost = PREVIEW.laser.energy;
      if (this.energy >= cost) {
        this.energy -= cost;
        this.fire(camera, PREVIEW.laser.speed, PREVIEW.laser.lifetime, PREVIEW.laser.color, 1.2);
      } else if (this.hull > 2 * cost) {
        this.hull -= 2 * cost; // overdrive gamble: 2 hull per 1 energy
        this.fire(camera, PREVIEW.laser.speed, PREVIEW.laser.lifetime, PREVIEW.laser.color, 1.2);
      }
      this.primaryCooldown = PREVIEW.laser.interval;
      this.lastWeapon = "Scrap Laser";
    }
    if (input.fireSecondary && this.secondaryCooldown <= 0 && this.scrap >= PREVIEW.rivet.scrap) {
      this.scrap -= PREVIEW.rivet.scrap;
      this.fire(camera, PREVIEW.rivet.speed, PREVIEW.rivet.lifetime, PREVIEW.rivet.color, 0.5);
      this.secondaryCooldown = PREVIEW.rivet.interval;
      this.lastWeapon = "Rivet Gun";
    }

    for (const d of this.drones) d.t = (((d.t + d.speed * dt) % 1) + 1) % 1;
    for (const p of this.projectiles) {
      p.pos = addScaled(p.pos, p.vel, dt);
      p.ttl -= dt;
    }
    this.projectiles = this.projectiles.filter((p) => p.ttl > 0);
  }

  ships(): Instance[] {
    return this.drones.map((d) => {
      const tangent = this.scene.pathTangent(d.t);
      const dir = d.speed >= 0 ? tangent : scale(tangent, -1);
      const rot = q.lookRotation(dir, { x: 0, y: 1, z: 0 });
      const offset = addScaled(addScaled(this.scene.pathPoint(d.t), q.right(rot), d.lateral), q.up(rot), d.vertical);
      return { position: offset, rotation: rot, scale: 1, color: TEAM_HULL[d.team] ?? TEAM_HULL[0]!, emissive: 0.1 };
    });
  }

  projectileInstances(): Instance[] {
    return this.projectiles.map((p) => ({ position: p.pos, rotation: p.rot, scale: p.scale, color: p.color, emissive: 1 }));
  }

  hud(): Partial<HudValues> {
    return {
      hull: this.hull,
      hullMax: PREVIEW.hullMax,
      energy: this.energy,
      energyMax: PREVIEW.energyMax,
      boost: this.boost,
      boostMax: PREVIEW.boostMax,
      scrap: this.scrap,
      weapon: this.lastWeapon,
      ammo: this.lastWeapon === "Rivet Gun" ? `${Math.floor(this.scrap)} scrap` : "∞",
      paint: this.scene.paintShare,
      timeLeft: this.timeLeft,
    };
  }

  private fire(camera: FlightCamera, speed: number, lifetime: number, color: Rgb, size: number): void {
    const rot = camera.orientation;
    const fwd = q.forward(rot);
    // Projectiles never inherit the shooter's velocity (design doc rule).
    const muzzle = addScaled(addScaled(camera.position, fwd, 1.2), q.up(rot), -0.35);
    this.projectiles.push({ pos: muzzle, vel: scale(fwd, speed), rot, ttl: lifetime, color, scale: size });
  }
}
