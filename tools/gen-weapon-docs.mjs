#!/usr/bin/env node
// Generates the player reference tables in userDocs/reference/ from the game data:
//   data/weapons/*.ron  -> userDocs/reference/weapons.md
//   data/ships/*.ron    -> userDocs/reference/ships.md
//   data/gadgets/*.ron  -> userDocs/reference/gadgets.md (with the Shield setting per weapon)
//   data/rulesets/*.ron -> userDocs/reference/modes.md (every built-in rule set: mode, teams,
//                          match length, mode settings, mutators, arsenal); the team counts each
//                          mode allows come from web/src/rules/catalog.ts (generated from ls-sim)
//   data/events/*.ron   -> userDocs/reference/events.md (the comedy director's cadence, every
//                          event with its numbers, moods and weight, each rule set's Absurdity
//                          cap and event bans)
// The default rule set (data/rulesets/paint_the_void.ron) supplies the weapon order (the
// design doc's) and the "default rules" columns.
//
// Usage: node tools/gen-weapon-docs.mjs [--check]
//   (no flag)  write the five files
//   --check    write nothing; exit 1 if a file is missing or differs from what the data
//              gives (CI runs this: after a data change, regenerate and commit the tables)
//
// The RON reader below understands the subset the data files use (structs, tuples, maps,
// lists, enums with and without payload, Some/None, strings, numbers, booleans, comments).

import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dataDir = join(repoRoot, "data");
const outDir = join(repoRoot, "userDocs", "reference");

// ---------------------------------------------------------------------------
// RON subset reader

/** An enum value: `Energy(3.0)` is { tag: "Energy", args: [3] }, `Primary` is { tag, args: [] }. */
class Enum {
  constructor(tag, args) {
    this.tag = tag;
    this.args = args;
  }
}

export function parseRon(text, source = "input") {
  let i = 0;
  const fail = (msg) => {
    const line = text.slice(0, i).split("\n").length;
    throw new Error(`${source}:${line}: ${msg}`);
  };
  const skip = () => {
    for (;;) {
      while (i < text.length && /\s/.test(text[i])) i++;
      if (text.startsWith("//", i)) {
        while (i < text.length && text[i] !== "\n") i++;
      } else if (text.startsWith("/*", i)) {
        const end = text.indexOf("*/", i + 2);
        if (end < 0) fail("unterminated comment");
        i = end + 2;
      } else return;
    }
  };
  const peek = () => {
    skip();
    return text[i];
  };
  const expect = (c) => {
    if (peek() !== c) fail(`expected '${c}', found '${text[i] ?? "end of file"}'`);
    i++;
  };
  const ident = () => {
    skip();
    const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(text.slice(i));
    if (!m) fail(`expected an identifier`);
    i += m[0].length;
    return m[0];
  };
  /** After '(' : either `name: value, ...` (a struct) or `value, ...` (a tuple). */
  const parenBody = () => {
    skip();
    const save = i;
    const m = /^[A-Za-z_][A-Za-z0-9_]*\s*:(?!:)/.exec(text.slice(i));
    if (m) {
      const obj = {};
      while (peek() !== ")") {
        const key = ident();
        expect(":");
        obj[key] = value();
        if (peek() === ",") i++;
        else break;
      }
      expect(")");
      return obj;
    }
    i = save;
    const items = [];
    while (peek() !== ")") {
      items.push(value());
      if (peek() === ",") i++;
      else break;
    }
    expect(")");
    return items;
  };
  const value = () => {
    const c = peek();
    if (c === undefined) fail("unexpected end of file");
    if (c === '"') {
      i++;
      let s = "";
      while (i < text.length && text[i] !== '"') {
        if (text[i] === "\\") {
          i++;
          const e = text[i];
          s += e === "n" ? "\n" : e === "t" ? "\t" : e;
        } else s += text[i];
        i++;
      }
      if (text[i] !== '"') fail("unterminated string");
      i++;
      return s;
    }
    if (c === "-" || c === "+" || (c >= "0" && c <= "9")) {
      const m = /^[-+]?\d+(\.\d*)?([eE][-+]?\d+)?/.exec(text.slice(i));
      if (!m) fail("bad number");
      i += m[0].length;
      return Number(m[0]);
    }
    if (c === "[") {
      i++;
      const items = [];
      while (peek() !== "]") {
        items.push(value());
        if (peek() === ",") i++;
        else break;
      }
      expect("]");
      return items;
    }
    if (c === "{") {
      i++;
      const map = new Map();
      while (peek() !== "}") {
        const k = value();
        expect(":");
        map.set(k, value());
        if (peek() === ",") i++;
        else break;
      }
      expect("}");
      return map;
    }
    if (c === "(") {
      i++;
      const body = parenBody();
      return Array.isArray(body) && body.length === 1 ? body[0] : body;
    }
    const name = ident();
    if (name === "true") return true;
    if (name === "false") return false;
    if (name === "None") return null;
    if (peek() === "(") {
      i++;
      const body = parenBody();
      if (name === "Some") return Array.isArray(body) ? body[0] : body;
      if (!Array.isArray(body)) return { __type: name, ...body };
      return new Enum(name, body);
    }
    return new Enum(name, []);
  };
  const v = value();
  skip();
  if (i < text.length) fail("trailing text after the value");
  return v;
}

const tagOf = (v) => (v instanceof Enum ? v.tag : String(v));

// ---------------------------------------------------------------------------
// Loading

function loadDir(sub) {
  const dir = join(dataDir, sub);
  return readdirSync(dir)
    .filter((f) => f.endsWith(".ron"))
    .sort()
    .map((f) => {
      const path = join(dir, f);
      return { file: `data/${sub}/${f}`, id: basename(f, ".ron"), def: parseRon(readFileSync(path, "utf8"), path) };
    });
}

// ---------------------------------------------------------------------------
// Formatting helpers

/** A number with at most `digits` decimals, trailing zeros dropped. */
export function num(v, digits = 2) {
  if (typeof v !== "number" || !Number.isFinite(v)) return "–";
  const r = Number(v.toFixed(digits));
  return String(Object.is(r, -0) ? 0 : r);
}

const pct = (f) => `${num(f * 100, 0)}%`;
const param = (def, key) => (def.params instanceof Map ? def.params.get(key) : undefined);

function table(headers, rows, align) {
  const a = align ?? headers.map(() => "---");
  const esc = (s) => String(s).replace(/\|/g, "\\|");
  return [
    `| ${headers.map(esc).join(" | ")} |`,
    `| ${a.join(" | ")} |`,
    ...rows.map((r) => `| ${r.map(esc).join(" | ")} |`),
  ].join("\n");
}

function header(title, sources, intro) {
  return [
    `# ${title}`,
    "",
    `<!-- Generated by tools/gen-weapon-docs.mjs from ${sources}. Do not edit by hand: change the`,
    `     data and run \`node tools/gen-weapon-docs.mjs\` (CI fails while this file is stale). -->`,
    "",
    intro,
    "",
  ].join("\n");
}

// ---------------------------------------------------------------------------
// Weapons

const FAMILY_TITLES = [
  ["Energy", "Energy primaries", "Powered by energy; bolts at about laser speed that a good pilot can dodge."],
  ["Kinetic", "Kinetic primaries", "Very fast rounds that fire your scrap, the same scrap that repairs your hull."],
  ["Ordnance", "Ordnance", "Missiles and mines with a limited count, found as pickups."],
  ["Special", "Specials", "The comedy weapons: rarer pickups, each with its own trick."],
];

const AVAILABILITY_TEXT = {
  LoadoutAndPickup: "start + pickup",
  PickupOnly: "pickup",
  LoadoutOnly: "start only",
  Banned: "banned",
};

function costText(w) {
  const c = w.cost;
  if (!(c instanceof Enum)) return "–";
  const v = c.args[0];
  if (c.tag === "Energy") return `${num(v)} energy`;
  if (c.tag === "Scrap") return `${num(v)} scrap`;
  if (c.tag === "Count") return w.max_ammo ? `${num(v)} (carry ${w.max_ammo})` : `${num(v)}`;
  return c.tag;
}

function shieldText(s) {
  if (!(s instanceof Enum)) return "–";
  const f = s.args[0];
  if (s.tag === "Blocked") return "blocked";
  if (s.tag === "Wears") return f === 1 ? "wears it" : `wears it ×${num(f)}`;
  if (s.tag === "PassThrough") return f >= 1 ? "passes through" : `${pct(f)} passes through`;
  return s.tag;
}

function rateText(interval) {
  if (!(interval > 0)) return "–";
  if (interval === 1) return "1/s";
  if (interval > 1) return `1 per ${num(interval)} s`;
  return `${num(1 / interval, 1)}/s`;
}

/** Direct damage text: base, with the level-max or full-charge value. */
function damageText(w) {
  const n = w.projectiles ?? 1;
  const base = n > 1 ? `${num(w.damage)} ×${n}` : num(w.damage);
  const extra = [];
  const maxLevel = w.max_level ?? 1;
  if (maxLevel > 1 && w.damage_per_level) {
    extra.push(`${num(w.damage * (1 + w.damage_per_level * (maxLevel - 1)), 1)} at level ${maxLevel}`);
  }
  const mult = w.charge?.damage_mult ?? 1;
  if (w.charge && mult !== 1) extra.push(`${num(w.damage * mult, 1)} charged`);
  return extra.length ? `${base} (${extra.join(", ")})` : base;
}

function splashText(s) {
  if (!s) return "–";
  return `${num(s.damage)} within ${num(s.radius)} u${s.hits_owner ? ", hurts you" : ""}`;
}

function lastingText(w) {
  const t = w.terrain ?? {};
  const parts = [];
  if (t.crater_radius > 0) parts.push(`crater Ø ${num(t.crater_radius * 2)} u`);
  if (t.paint_radius > 0) parts.push(`paint Ø ${num(t.paint_radius * 2)} u`);
  return parts.join(", ") || "–";
}

/**
 * Notes from a weapon's own parameters (`params` names in data/weapons/*.ron): what the
 * numbers in the parameter table mean for a pilot. A weapon without them gets none.
 */
const PARAM_NOTES = [
  [["drain_per_arc", "range"], (p) => `instant arc to a target within ${num(p.range)} u, drains ${num(p.drain_per_arc)} energy per arc`],
  [["overcharge_hull_per_sec"], (p) => `holding past full charge burns ${num(p.overcharge_hull_per_sec)} hull/s`],
  [["bounces"], (p) => `bounces off walls up to ${num(p.bounces)} times`],
  [["falloff_min", "falloff_end"], (p) => `damage falls to ${pct(p.falloff_min)} by ${num(p.falloff_end)} u`],
  [["falloff_min"], (p) => `damage falls to ${pct(p.falloff_min)} towards the end of the bolt's life`],
  [["pierce_thickness"], (p) => `pierces ships, debris and walls up to ${num(p.pierce_thickness)} u thick`],
  [["cone_deg", "range"], (p) => `homes on targets in a ${num(p.cone_deg)}° cone within ${num(p.range)} u`],
  [["blobs", "blob_damage"], (p) => `releases ${num(p.blobs)} homing blobs of ${num(p.blob_damage)} damage on impact`],
  [["blind_radius", "blind_secs"], (p) => `blinds pilots within ${num(p.blind_radius)} u for ${num(p.blind_secs)} s`],
  [["detect_radius", "creep_speed", "arm_secs", "trigger_radius"], (p) => `arms after ${num(p.arm_secs)} s, creeps at ${num(p.creep_speed)} u/s toward enemies within ${num(p.detect_radius)} u, goes off within ${num(p.trigger_radius)} u`],
  [["arm_secs", "trigger_radius"], (p) => `arms after ${num(p.arm_secs)} s, goes off within ${num(p.trigger_radius)} u`],
  [["core_radius", "core_damage"], (p) => `everything within ${num(p.core_radius)} u dies; ${num(p.sub_charges ?? 0)} sub-charges of ${num(p.sub_damage ?? 0)} splash follow`],
  [["return_after_secs"], (p) => `flies back along its path after ${num(p.return_after_secs)} s`],
  [["rewind_secs", "offline_secs"], (p) => `rewinds you ${num(p.rewind_secs)} s; weapons offline ${num(p.offline_secs)} s`],
  [["cone_half_deg", "range"], (p) => `pulls things in a ${num(p.cone_half_deg * 2)}° cone within ${num(p.range)} u`],
  [["duck_chance"], (p) => `${num(p.duck_chance * 100, 1)}% chance of a giant rubber duck`],
  [["well_secs", "well_range", "well_pull"], (p) => `stops where it lands; a gravity well for ${num(p.well_secs)} s pulls ships (yours too), debris and ordnance within ${num(p.well_range)} u at up to ${pct(p.well_pull)} of a Tinker's thrust, then fuses into a lump of weak rock`],
];

/** Instant weapons (the Leech Arc) reach their `range` parameter at once, no projectile flies. */
function isInstant(w) {
  return param(w, "drain_per_arc") !== undefined && param(w, "range") !== undefined;
}

/** Mines (they arm and wait) have no useful "range". */
function isMine(w) {
  return param(w, "arm_secs") !== undefined;
}

function notesText(w) {
  const notes = [];
  if (w.params instanceof Map) {
    const p = Object.fromEntries(w.params);
    const used = new Set();
    for (const [keys, text] of PARAM_NOTES) {
      if (keys.every((k) => k in p) && !keys.some((k) => used.has(k))) {
        notes.push(text(p));
        for (const k of keys) used.add(k);
      }
    }
  }
  const n = w.projectiles ?? 1;
  if (n > 1) notes.push(`${n} projectiles${w.spread_deg ? `, ${num(w.spread_deg)}° apart` : ""}`);
  if ((w.max_level ?? 1) > 1) notes.push(`upgrades to level ${w.max_level}`);
  if (w.charge) {
    const c = w.charge;
    const bits = [`hold to charge, full at ${num(c.max_secs)} s`];
    if (c.min_secs) bits.push(`no shot before ${num(c.min_secs)} s`);
    if (c.auto_release) bits.push("fires by itself");
    notes.push(bits.join(", "));
  }
  if (w.gravity === true) notes.push("gravity wells bend it");
  return notes.join("; ") || "–";
}

/** Direct hits (whole shots, level 1, no charge) to destroy a ship with `hull`. */
function hitsToKill(w, hull) {
  const perShot = w.damage * (w.projectiles ?? 1);
  if (!(perShot > 0)) return "–";
  return String(Math.ceil(hull / perShot - 1e-9));
}

function weaponsDoc(weapons, rules, tinkerHull) {
  const order = rules ? [...rules.weapons.keys()] : [];
  const rank = (id) => {
    const k = order.indexOf(id);
    return k < 0 ? order.length : k;
  };
  const sorted = [...weapons].sort((a, b) => rank(a.id) - rank(b.id) || a.id.localeCompare(b.id));
  const out = [
    header(
      "Weapon reference",
      "data/weapons/*.ron and data/rulesets/paint_the_void.ron",
      [
        "Every weapon's numbers as this build's data files define them. What each weapon does, its",
        "malus and its lasting effect are in the [player manual](../player-manual.md#weapons).",
        "",
        "How to read the tables:",
        "",
        "- **Damage** is per direct hit at level 1 before the ship's damage multiplier (Skitter",
        "  0.85x, Tinker 1x, Hauler 1.2x, Tug 0.8x, see [ships](ships.md)). `×N` means N projectiles",
        "  per shot. Friendly fire scales damage between teammates (50% by default).",
        `- **Hits to kill** is the number of whole shots that destroy a full-hull Tinker (${num(tinkerHull)} hull)`,
        "  with direct hits only (level 1, no charge, no splash).",
        "- **Range** is speed times lifetime. Projectiles never inherit your ship's speed.",
        "- **Splash** is the most area damage (at the centre, falling off to 0 at the radius).",
        "- **Crater** sizes are diameters on ordinary rock. Weak rock (light, sandy) blasts 1.4x",
        "  wider, hard rock (dark, glinting) 0.6x.",
        "- **Shield** is how the weapon meets an enemy's active [Shield](gadgets.md#shield-and-weapons):",
        "  blocked (the shield's energy pays, never the hull), wears it (energy pays, the hull takes",
        "  what the energy cannot cover) or a share that passes through to the hull.",
        "- **Default rules** is how Paint the Void (the built-in rule set) hands the weapon out.",
        "  A server's rule set can change availability, pickups, ammo, damage and fire rate.",
      ].join("\n"),
    ),
  ];
  for (const [family, title, blurb] of FAMILY_TITLES) {
    const list = sorted.filter((w) => tagOf(w.def.family) === family);
    if (list.length === 0) continue;
    out.push(`## ${title}`, "", blurb, "");
    out.push(
      table(
        ["Weapon", "Slot", "Damage", "Hits to kill", "Fire rate", "Cost per shot", "Speed (u/s)", "Range (u)", "Splash", "Knockback (u/s)", "Crater / paint", "Shield", "Default rules"],
        list.map(({ id, def: w }) => {
          const avail = rules ? tagOf(rules.weapons.get(id) ?? new Enum("Banned", [])) : "";
          const pickup = rules?.pickups.get(id);
          const ammo = pickup?.ammo ? `, ${pickup.ammo} per pickup` : "";
          const start = rules?.starting_loadout.secondaries.find(([sid]) => sid === id);
          const startText = start ? `, start with ${start[1]}` : "";
          return [
            `**${w.name}**`,
            tagOf(w.slot).toLowerCase(),
            damageText(w),
            hitsToKill(w, tinkerHull),
            rateText(w.fire_interval_secs),
            costText(w),
            isInstant(w) ? "instant" : num(w.speed),
            isInstant(w) ? num(param(w, "range")) : isMine(w) ? `mine, lasts ${num(w.lifetime_secs)} s` : num(w.speed * w.lifetime_secs, 1),
            splashText(w.splash),
            num(w.knockback ?? 0),
            lastingText(w),
            shieldText(w.shield),
            (AVAILABILITY_TEXT[avail] ?? avail) + startText + ammo,
          ];
        }),
      ),
      "",
    );
    const noted = list.filter(({ def }) => notesText(def) !== "–");
    if (noted.length) {
      out.push(...noted.map(({ def }) => `- **${def.name}:** ${notesText(def)}.`), "");
    }
  }
  out.push(
    "## Weapon parameters",
    "",
    "Numbers each weapon's own code reads from its data file (`params`). Durations are in seconds,",
    "distances in u, speeds in u/s, angles in degrees. They are listed for hosts and curious",
    "pilots; the [player manual](../player-manual.md#weapons) explains what they do.",
    "",
    table(
      ["Weapon", "Parameters"],
      sorted
        .filter(({ def }) => def.params instanceof Map && def.params.size > 0)
        .map(({ def }) => [def.name, [...def.params].map(([k, v]) => `\`${k}\` ${num(v, 3)}`).join(", ")]),
    ),
    "",
  );
  return out.join("\n");
}

// ---------------------------------------------------------------------------
// Ships

const SHIP_ORDER = ["skitter", "tinker", "hauler", "tug"];
const SHIP_ROLES = {
  skitter: "Light: hit and run, smallest hitbox, fastest turning",
  tinker: "Standard: the all-rounder",
  hauler: "Heavy: holds tunnels and soaks damage, largest hitbox",
  tug: "Support: scavenges faster and passes scrap to teammates",
};
const LASER_SPEED = 30;

function shipsDoc(ships) {
  const sorted = [...ships].sort((a, b) => {
    const ra = SHIP_ORDER.indexOf(a.id);
    const rb = SHIP_ORDER.indexOf(b.id);
    return (ra < 0 ? 99 : ra) - (rb < 0 ? 99 : rb) || a.id.localeCompare(b.id);
  });
  const name = (id) => id.charAt(0).toUpperCase() + id.slice(1);
  const diag = (v) => v * Math.sqrt(3);
  const out = [
    header(
      "Ship reference",
      "data/ships/*.ron",
      [
        "The numbers of the four ship types in this build. How to pick one and how each plays is in",
        "the [player manual](../player-manual.md#ship-types). Speeds are in u/s (1 u = one ship",
        `length); a Scrap Laser bolt flies at ${LASER_SPEED} u/s. Thrust is capped per axis, so flying`,
        "diagonally on all three axes is 1.73 times (the square root of 3) faster than flying straight.",
      ].join("\n"),
    ),
    "## Overview",
    "",
    table(
      ["Ship", "Role", "Hull", "Damage dealt", "Secondary slots", "Top speed", "Hitbox radius (u)", "Wreck scrap"],
      sorted.map(({ id, def: s }) => [
        `**${name(id)}**`,
        SHIP_ROLES[id] ?? "",
        num(s.hull_max),
        `${num(s.damage_mult)}x`,
        num(s.secondary_slots),
        `${num(diag(s.boost_speed), 1)} u/s (${pct(diag(s.boost_speed) / LASER_SPEED)} of laser)`,
        num(s.radius),
        num(s.wreck_scrap),
      ]),
    ),
    "",
    "**Top speed** is boosting diagonally on all three axes. **Damage dealt** multiplies the damage",
    "and knockback of every weapon the ship fires. **Wreck scrap** is what the wreck holds when the",
    "ship is destroyed, plus half the scrap it carried.",
    "",
    "## Flight",
    "",
    table(
      ["Ship", "Straight, cruise", "Straight, boost", "3-axis diagonal, cruise", "3-axis diagonal, boost", "Thrust (u/s²)", "Brake (u/s²)", "Turn rate (rad/s)", "Roll rate (rad/s)"],
      sorted.map(({ id, def: s }) => [
        `**${name(id)}**`,
        num(s.cruise_speed),
        num(s.boost_speed),
        num(diag(s.cruise_speed), 1),
        num(diag(s.boost_speed), 1),
        num(s.thrust_accel),
        num(s.brake_accel),
        num(s.max_turn_rate),
        num(s.roll_rate),
      ]),
    ),
    "",
    "## Resources",
    "",
    table(
      ["Ship", "Energy (max / at spawn)", "Energy regen (/s)", "Boost (max / at spawn)", "Boost drain (/s)", "Scrap (max / at spawn)", "Scavenge reach (u)", "Scavenge rate (/s)", "Hull per scrap", "Overdrive (hull per energy)"],
      sorted.map(({ id, def: s }) => [
        `**${name(id)}**`,
        `${num(s.energy_max)} / ${num(s.spawn_energy)}`,
        num(s.energy_regen),
        `${num(s.boost_max)} / ${num(s.spawn_boost)}`,
        num(s.boost_drain),
        `${num(s.scrap_max)} / ${num(s.spawn_scrap)}`,
        num(s.scavenge_radius),
        num(s.scavenge_rate),
        num(s.hull_per_scrap),
        num(s.overdrive_hull_per_energy),
      ]),
    ),
    "",
    "Energy regenerates only while you are not boosting. Hull never regenerates: only scrap repairs",
    "it (scavenging, or a Tug passing scrap to you).",
    "",
    "## Boost refills and special abilities",
    "",
    table(
      ["Ship", "Hard brake: refill per u/s braked", "Hard brake from (u/s)", "Wall scrape: max refill (/s)", "Scrape within (u)", "Scrape from (u/s)", "Passes scrap to teammates"],
      sorted.map(({ id, def: s }) => [
        `**${name(id)}**`,
        num(s.brake_refill_per_speed),
        num(s.hard_brake_min_speed),
        num(s.scrape_refill),
        num(s.scrape_distance),
        num(s.scrape_min_speed),
        s.can_pass_scrap ? `yes, ${num(s.scrap_pass_rate)} scrap/s (hold R within ${num(s.scavenge_radius)} u)` : "no",
      ]),
    ),
    "",
    "## Paint effects",
    "",
    table(
      ["Ship", "Wall bounce (restitution)", "On red (bouncy) paint", "On yellow (sticky) paint"],
      sorted.map(({ id, def: s }) => [
        `**${name(id)}**`,
        num(s.restitution),
        num(s.paint_bounce_restitution ?? 0.9),
        `speed held to ${pct(s.paint_sticky_speed ?? 0.4)}`,
      ]),
    ),
    "",
    "Blue (slippery) paint has no number: while touching it the ship is not slowed down, and a",
    "wall hit keeps the speed, turned along the wall.",
    "",
  ];
  return out.join("\n");
}

// ---------------------------------------------------------------------------
// Gadgets

const GADGET_ORDER = ["shield", "cloak", "decoy"];

function gadgetsDoc(gadgets, weapons, rules) {
  const sorted = [...gadgets].sort((a, b) => GADGET_ORDER.indexOf(a.id) - GADGET_ORDER.indexOf(b.id) || a.id.localeCompare(b.id));
  const order = rules ? [...rules.weapons.keys()] : [];
  const sortedWeapons = [...weapons].sort((a, b) => {
    const ra = order.indexOf(a.id);
    const rb = order.indexOf(b.id);
    return (ra < 0 ? 999 : ra) - (rb < 0 ? 999 : rb) || a.id.localeCompare(b.id);
  });
  const out = [
    header(
      "Gadget reference",
      "data/gadgets/*.ron and data/weapons/*.ron",
      [
        "One gadget slot per ship; **G** uses it. How each gadget plays is in the",
        "[player manual](../player-manual.md#gadgets).",
      ].join("\n"),
    ),
    "## Gadgets",
    "",
    table(
      ["Gadget", "Use", "Energy to use", "Drain while on (/s)", "Drain while moving", "Cooldown (s)", "Lasts (s)", "Default rules"],
      sorted.map(({ id, def: g }) => {
        const avail = rules ? tagOf(rules.gadgets.get(id) ?? new Enum("Banned", [])) : "";
        const loadout = rules?.starting_loadout.gadget === id;
        return [
          `**${g.name}**`,
          tagOf(g.activation) === "Toggle" ? "toggle on/off" : "instant",
          num(g.energy_cost ?? 0),
          g.drain_per_sec ? num(g.drain_per_sec) : "–",
          (g.moving_drain_mult ?? 1) !== 1 ? `${num(g.moving_drain_mult)}x` : "–",
          num(g.cooldown_secs ?? 0),
          g.duration_secs ? num(g.duration_secs) : "–",
          (AVAILABILITY_TEXT[avail] ?? avail) + (loadout ? " (everyone starts with it)" : ""),
        ];
      }),
    ),
    "",
    "## Gadget parameters",
    "",
    table(
      ["Gadget", "Parameters"],
      sorted
        .filter(({ def }) => def.params instanceof Map && def.params.size > 0)
        .map(({ def }) => [def.name, [...def.params].map(([k, v]) => `\`${k}\` ${num(v, 3)}`).join(", ")]),
    ),
    "",
  ];
  const shield = sorted.find(({ id }) => id === "shield")?.def;
  if (shield) {
    const selfCut = param(shield, "self_damage_reduction");
    const perDamage = param(shield, "energy_per_damage") ?? 1;
    out.push(
      "## Shield and weapons",
      "",
      `An active Shield cuts damage from your own weapons by ${selfCut !== undefined ? pct(selfCut) : "a share"}. Against enemy fire`,
      "each weapon has its own setting (a rule set can override it per weapon):",
      "",
      `- **blocked:** the shield pays ${num(perDamage)} energy per damage point; the hull never takes damage from`,
      "  it, even when the energy is gone.",
      `- **wears it (×f):** the shield pays f × ${num(perDamage)} energy per damage point; what the energy cannot`,
      "  cover reaches the hull.",
      "- **N% passes through:** that share reaches the hull; the shield absorbs the rest for free.",
      "",
      "Knockback is never reduced. The shield drops when your energy runs out.",
      "",
      table(
        ["Weapon", "Against an enemy's Shield"],
        sortedWeapons.map(({ def: w }) => [w.name, shieldText(w.shield)]),
      ),
      "",
    );
  }
  return out.join("\n");
}

// ---------------------------------------------------------------------------
// Modes

/**
 * The modes in `ls_sim::ModeKind` order: RON tag, display name, what it is about and how a
 * match is won (texts for pilots; the numbers come from the rule sets).
 */
const MODES = [
  {
    tag: "PaintTheVoid",
    name: "Paint the Void",
    anchor: "paint-the-void",
    about: "Teams paint the level's surfaces in their colour with the Paint Thrower; craters erase paint. Kills don't score.",
    wins: () => "most painted surface when the timer ends (equal: draw)",
  },
  {
    tag: "TeamBrawl",
    name: "Team Brawl",
    anchor: "team-brawl",
    about: "Team deathmatch with respawns, or every pilot for themselves with 0 teams (the duel). A kill scores 1; a suicide or team kill costs the side points.",
    wins: (m) =>
      m.kill_target > 0
        ? `first to ${m.kill_target} kills, else most kills on time (equal: draw)`
        : "first to the kill target (15 to 50, from the player count), else most kills on time (equal: draw)",
  },
  {
    tag: "TheSqueeze",
    name: "The Squeeze",
    anchor: "the-squeeze",
    about: "Every pilot for themselves with one life. Walls grow inward from the level's edge in phases (a warning, then the close) and crush ships caught in them; closing walls shed scrap.",
    wins: () => "last ship flying (ships lost in the same tick: draw)",
  },
  {
    tag: "Heist",
    name: "Heist",
    anchor: "heist",
    about: "Dig the core out of its vein at the bottom of the level and carry it to your extraction zone. The carrier is slower and can't boost; a dropped core returns to the vein. Kills don't score.",
    wins: (m) => `first to ${m.deliveries_to_win} deliveries, else most deliveries on time (equal: draw)`,
  },
  {
    tag: "HotReactor",
    name: "Hot Reactor",
    anchor: "hot-reactor",
    about: "A reactor sits on one pilot and passes to whoever hits or rams them. Its fuse runs faster the more ships are near; whoever holds it at detonation loses a life.",
    wins: (m) => `last pilot with lives (${m.lives} each), else most lives on time`,
  },
  {
    tag: "LastColorStanding",
    name: "Last Color Standing",
    anchor: "last-color-standing",
    about: "A black-and-white level; every pilot has a colour meter that each death drains. Out of colour, out of the match. A late-match stalemate drain makes sure it ends.",
    wins: () => "last pilot with colour, else most colour on time",
  },
  {
    tag: "TheArchitect",
    name: "The Architect",
    anchor: "the-architect",
    about: "One pilot is the architect: no ship, a budget of strokes that reshape the level live (grow or open rock, pillars, a gravity well, a cave-in, a comedy event, an ink blackout). Everyone else runs from one end of the level to the exit at the other. The role rotates every round; runners never hurt each other.",
    wins: (m) =>
      `most points after min(players, ${m.max_rounds}) rounds of ${duration(m.round_secs)}: 1 per runner through the exit, 1 to the architect per runner who misses (equal: draw)`,
  },
  {
    tag: "BeltUp",
    name: "Belt Up",
    anchor: "belt-up",
    about: "A race on an asteroid belt level, in heats: one lap through the 8 gates in order, then a dive to the EXIT at the centre. The belt shields you from radiation but is full of rock and drifting boulders; open space is fast but burns hull. A destroyed ship goes back to its last gate with the hull it had there. Ships bump each other; kills don't score.",
    wins: (m) => `most points over ${m.heats} heats (${[1, 2, 3, 4, 5, 6, 7, 8].map((p) => m[`points_${p}`]).join(", ")} per place), then more heat wins, the better last place, the lower total time`,
  },
];

/** What each mode setting means (`ModeRules` fields in `crates/ls-sim/src/ruleset.rs`) and how to print it. */
const MODE_FIELDS = {
  teams: ["Teams (0 = every pilot for themselves)", (v) => (v === 0 ? "free-for-all" : String(v))],
  match_secs: ["Match length (the timer)", (v) => duration(v)],
  respawn_secs: ["Wait after being destroyed (s)", (v) => (v > 0 ? num(v) : "no respawn")],
  kill_target: ["Kills to win (0 = max(15, 5 × players / teams), at most 50)", (v) => (v > 0 ? String(v) : "scaled")],
  team_kill_penalty: ["Points a suicide or a team kill costs the side", (v) => String(v)],
  first_phase_secs: ["First wall warning after the start (s)", (v) => num(v)],
  phases: ["Wall phases", (v) => String(v)],
  warn_secs: ["Warning before each close (s)", (v) => num(v)],
  close_secs: ["Each close takes (s)", (v) => num(v)],
  crush_per_sec: ["Crushing damage while touching a moving wall (hull/s, ignores shields)", (v) => num(v)],
  scrap_per_100u2: ["Scrap debris pieces per 100 u² of closed opening", (v) => num(v)],
  deliveries_to_win: ["Deliveries to win", (v) => String(v)],
  carrier_speed_percent: ["Carrier's top speed (no boost)", (v) => `${v}%`],
  drop_return_secs: ["A dropped core returns to the vein after (s)", (v) => num(v)],
  core_respawn_secs: ["A delivered core is back in the vein after (s)", (v) => num(v)],
  collapse_secs: ["The mine sheds its ceiling for (s) once the core is free", (v) => num(v)],
  lives: ["Lives (lost only to a detonation in your hands)", (v) => String(v)],
  fuse_secs: ["Fuse at normal speed (s)", (v) => num(v)],
  fuse_radius: ["Ships within this distance of the holder speed the fuse up (u)", (v) => num(v)],
  fuse_speed_per_ship: ["Extra fuse speed per ship near the holder", (v) => `+${num(v)}x`],
  blast_radius: ["Detonation blast radius (u)", (v) => num(v)],
  blast_damage: ["Detonation blast damage at the centre", (v) => num(v)],
  pass_immunity_secs: ["A new holder can't lose the reactor for (s)", (v) => num(v)],
  next_reactor_secs: ["Next reactor appears after (s)", (v) => num(v)],
  colour: ["Colour at the start", (v) => String(v)],
  drain_per_kill: ["Colour each death drains", (v) => String(v)],
  stalemate_secs: ["Stalemate drain starts after", (v) => duration(v)],
  stalemate_drain_secs: ["Stalemate: 1 colour lost every (s)", (v) => num(v)],
  round_secs: ["Round length", (v) => duration(v)],
  round_results_secs: ["Round results between rounds (s)", (v) => num(v)],
  max_rounds: ["Rounds (at most; fewer with fewer players)", (v) => String(v)],
  runner_lives: ["Runner lives per round", (v) => String(v)],
  budget_start: ["Architect's stroke budget at the round start", (v) => String(v)],
  budget_per_sec: ["Budget gained per second", (v) => num(v)],
  budget_max: ["Largest budget", (v) => String(v)],
  stroke_cooldown_secs: ["Time between two strokes (s)", (v) => num(v)],
  telegraph_secs: ["A stroke glows for (s) before it acts", (v) => num(v)],
  max_strokes: ["Strokes acting at once", (v) => String(v)],
  min_radius: ["Smallest stroke radius (u)", (v) => num(v)],
  max_radius: ["Largest stroke radius (u)", (v) => num(v)],
  keep_clear: ["Rock strokes keep this far from the exit and the start (u)", (v) => num(v)],
  cost_enclosure_plus: ["Cost: Enclosure+ (grow rock)", (v) => String(v)],
  cost_enclosure_minus: ["Cost: Enclosure- (open rock)", (v) => String(v)],
  cost_density_plus: ["Cost: Density+ (pillars, floating rocks)", (v) => String(v)],
  cost_gravity_plus: ["Cost: Gravity Chaos+ (a gravity well)", (v) => String(v)],
  cost_instability_plus: ["Cost: Instability+ (a cave-in)", (v) => String(v)],
  cost_absurdity_plus: ["Cost: Absurdity+ (a random comedy event)", (v) => String(v)],
  cost_palette: ["Cost: Palette (ink blackout of the region)", (v) => String(v)],
  cost_verticality: ["Cost: Verticality (the level gets bored and turns; once per round)", (v) => String(v)],
  reshape_delta: ["Rock strokes reshape like the slider moved by", (v) => String(v)],
  well_secs: ["Gravity Chaos+ well lasts (s)", (v) => num(v)],
  palette_secs: ["Palette blackout lasts (s)", (v) => num(v)],
  heats: ["Heats per match", (v) => String(v)],
  heat_limit_secs: ["A heat ends after (from the start lights going out)", (v) => duration(v)],
  finish_grace_secs: ["...or this long after the first racer finished (s)", (v) => num(v)],
  lights_secs: ["Start lights (ships held in their slots, s)", (v) => num(v)],
  results_secs: ["Heat results before the next heat (s)", (v) => num(v)],
  hold_secs: ["A destroyed ship is held at its gate until this long after its death (s)", (v) => num(v)],
  hull_floor_percent: ["Hull after a respawn: as at the last gate, at least", (v) => `${v}%`],
  radiation_scale: ["Radiation dose (x the level's law)", (v) => `${num(v)}x`],
  points_1: ["Points for 1st place", (v) => String(v)],
  points_2: ["Points for 2nd place", (v) => String(v)],
  points_3: ["Points for 3rd place", (v) => String(v)],
  points_4: ["Points for 4th place", (v) => String(v)],
  points_5: ["Points for 5th place", (v) => String(v)],
  points_6: ["Points for 6th place", (v) => String(v)],
  points_7: ["Points for 7th place", (v) => String(v)],
  points_8: ["Points for 8th place", (v) => String(v)],
  bump_restitution: ["Bumps: touching ships part at this times their approach speed", (v) => `${num(v)}x`],
  bump_max_speed: ["Bumps: at most this speed change per ship (u/s)", (v) => num(v)],
  bump_cooldown_secs: ["Bumps: once per pair in (s)", (v) => num(v)],
};

/** Mutators: name and what they change. */
const MUTATOR_TEXT = {
  Instagib:
    "Railspike only, fired for free; any direct enemy hit kills, shields or not. No secondaries, gadgets or pickups (Paint the Void keeps its Paint Thrower). Hull and energy are hidden, boost stays; respawns are instant.",
};

/** Rule set files in the order the reference lists them; others follow by name. */
const RULESET_ORDER = ["paint_the_void", "team_brawl", "squeeze", "heist", "hot_reactor", "last_color_standing", "the_architect", "belt_up", "instagib_brawl", "duel", "sandbox_all_weapons"];

/** `300` -> "5 min", `90` -> "1 min 30 s", `45` -> "45 s". */
export function duration(secs) {
  if (!(secs > 0)) return "–";
  const m = Math.floor(secs / 60);
  const s = Math.round(secs - m * 60);
  if (m === 0) return `${s} s`;
  return s ? `${m} min ${s} s` : `${m} min`;
}

/**
 * The mode list of web/src/rules/catalog.ts (generated from `ls_sim` by the rulecode test): per
 * mode kind the team counts it allows. Empty when the catalog is missing or unreadable.
 */
export function catalogModes(text) {
  const out = new Map();
  for (const line of text.split("\n")) {
    const m = /^\s*(\{"kind": .*\}),?\s*$/.exec(line);
    if (!m) continue;
    try {
      const v = JSON.parse(m[1]);
      if (typeof v.kind === "string" && Array.isArray(v.teams)) out.set(v.kind, v);
    } catch {
      /* not a mode line */
    }
  }
  return out;
}

function teamCountsText(counts) {
  if (!counts?.length) return "–";
  const parts = counts.map((t) => (t === 0 ? "free-for-all" : String(t)));
  const teams = parts.filter((p) => p !== "free-for-all");
  const text = teams.length ? `${teams.join(", ")} teams` : "";
  return [parts.includes("free-for-all") ? "free-for-all" : "", text].filter(Boolean).join(" or ");
}

function loadoutText(rules, names) {
  const lo = rules.starting_loadout ?? {};
  const parts = [...(lo.primaries ?? []).map((id) => names.get(id) ?? id)];
  for (const [id, n] of lo.secondaries ?? []) parts.push(`${n} ${names.get(id) ?? id}${n === 1 ? "" : "s"}`);
  if (lo.gadget) parts.push(names.get(lo.gadget) ?? lo.gadget);
  return parts.join(", ") || "nothing";
}

function pickupText(rules, names, weaponCount) {
  const w = rules.pickups instanceof Map ? [...rules.pickups.keys()] : [];
  const parts = [];
  if (w.length === 0) parts.push("no weapons");
  else if (w.length <= 3) parts.push(w.map((id) => names.get(id) ?? id).join(", "));
  else parts.push(`${w.length} of ${weaponCount} weapons`);
  const up = rules.upgrade_pickups instanceof Map ? [...rules.upgrade_pickups.keys()] : [];
  if (up.length) parts.push(`${up.map((id) => names.get(id) ?? id).join(", ")} upgrades`);
  const g = rules.gadget_pickups instanceof Map ? [...rules.gadget_pickups.keys()] : [];
  if (g.length) parts.push(g.map((id) => names.get(id) ?? id).join(", "));
  if (rules.scrap_pickup) parts.push("scrap piles");
  return w.length === 0 && !up.length && !g.length && !rules.scrap_pickup ? "none" : parts.join("; ");
}

function modesDoc(rulesets, weapons, gadgets, catalog) {
  const names = new Map([...weapons, ...gadgets].map(({ id, def }) => [id, def.name]));
  const rank = (id) => {
    const k = RULESET_ORDER.indexOf(id);
    return k < 0 ? RULESET_ORDER.length : k;
  };
  const sorted = [...rulesets].sort((a, b) => rank(a.id) - rank(b.id) || a.id.localeCompare(b.id));
  const modeOf = (def) => (def.mode instanceof Enum ? def.mode : new Enum("?", [{}]));
  const settings = (def) => modeOf(def).args[0] ?? {};
  const mutators = (def) => (Array.isArray(def.mutators) ? def.mutators.map(tagOf) : []);
  const info = (tag) => MODES.find((m) => m.tag === tag);
  const out = [
    header(
      "Mode reference",
      "data/rulesets/*.ron and web/src/rules/catalog.ts",
      [
        "Every built-in rule set of this build: its mode, teams, match length, the mode's settings,",
        "mutators and arsenal. How each mode plays is in the",
        "[player manual](../player-manual.md#game-modes); hosts pick rule sets by id (the file name",
        "without `.ron`) with `--ruleset <id>` or in a playlist, see the",
        "[hosting guide](../hosting-guide.md#playlists). A server's own rule set file or `LR` code can",
        "change every number here.",
      ].join("\n"),
    ),
    "## Built-in rule sets",
    "",
    table(
      ["Rule set (id)", "Mode", "Teams", "Match length", "Respawn", "Mutators", "How to win"],
      sorted.map(({ id, def }) => {
        const mode = modeOf(def);
        const m = settings(def);
        const mi = info(mode.tag);
        return [
          `**${def.name}** (\`${id}\`)`,
          mi ? `[${mi.name}](#${mi.anchor})` : mode.tag,
          MODE_FIELDS.teams[1](m.teams),
          duration(m.match_secs),
          m.respawn_secs > 0 ? `${num(m.respawn_secs)} s` : "none (one life)",
          mutators(def).join(", ") || "–",
          mi ? mi.wins(m) : "–",
        ];
      }),
    ),
    "",
    "Which of them a server plays, in which order and for how many players is its playlist",
    "([hosting guide](../hosting-guide.md#lobby-match-end-and-rotation)).",
    "",
  ];
  for (const mi of MODES) {
    const sets = sorted.filter(({ def }) => modeOf(def).tag === mi.tag);
    const cat = catalog.get(mi.tag);
    out.push(`## ${mi.name}`, "", mi.about, "");
    out.push(`- **Teams the mode allows:** ${teamCountsText(cat?.teams)}.`);
    if (sets.length === 0) {
      out.push("- No built-in rule set plays this mode.", "");
      continue;
    }
    out.push(`- **Built-in rule sets:** ${sets.map(({ id, def }) => `${def.name} (\`${id}\`)`).join(", ")}.`, "");
    const keys = [];
    for (const { def } of sets) for (const k of Object.keys(settings(def))) if (!keys.includes(k)) keys.push(k);
    out.push(
      table(
        ["Setting", ...sets.map(({ def }) => def.name), "Meaning"],
        keys.map((k) => {
          const [meaning, fmt] = MODE_FIELDS[k] ?? ["–", (v) => (typeof v === "number" ? num(v) : String(v))];
          return [`\`${k}\``, ...sets.map(({ def }) => (settings(def)[k] === undefined ? "–" : fmt(settings(def)[k]))), meaning];
        }),
      ),
      "",
    );
  }
  out.push("## Mutators", "", "Mutators combine with any mode; a rule set lists them in `mutators`.", "");
  out.push(
    table(
      ["Mutator", "What it changes", "Built-in rule sets with it"],
      Object.entries(MUTATOR_TEXT).map(([tag, text]) => {
        const using = sorted.filter(({ def }) => mutators(def).includes(tag)).map(({ id }) => `\`${id}\``);
        return [`**${tag}**`, text, using.join(", ") || "–"];
      }),
    ),
    "",
  );
  out.push(
    "## Arsenal per rule set",
    "",
    "What you start with and what spawns at the level's pickup points. Weapon numbers are in the",
    "[weapon reference](weapons.md), gadgets in the [gadget reference](gadgets.md).",
    "",
    table(
      ["Rule set", "Start with", "Pickups", "Friendly fire", "Ship types"],
      sorted.map(({ def }) => [
        `**${def.name}**`,
        loadoutText(def, names),
        pickupText(def, names, weapons.length),
        def.friendly_fire_percent ? `${def.friendly_fire_percent}%` : "off",
        (def.allowed_ship_types ?? []).map(tagOf).join(", ") || "–",
      ]),
    ),
    "",
    "Friendly fire only matters in team modes (in free-for-all every other ship is an enemy).",
    "",
  );
  return out.join("\n");
}

// ---------------------------------------------------------------------------
// Comedy events (M4)

/** Mood names as players see them (`data/events/*.ron` `moods`). */
const MOOD_NAMES = { Calm: "calm", Bored: "bored", Grumpy: "grumpy", Giddy: "giddy" };

/**
 * What each event does, for players, from its data (`p` = params, `d` = the
 * definition). The rules are in docs/milestone-4-plan.md §3.2 and the event
 * modules; an event without an entry here shows its data file's name only.
 */
const EVENT_TEXT = {
  bored_level: (p) =>
    `The level gets bored and turns its "up" ${num(p.turn_degrees)} degrees over ${num(p.turn_secs)} s, and stays turned: the sky, the sun and horizon assist follow. Nothing moves physically.`,
  tax_drone: (p) =>
    `A bureaucrat drone chases the leader at ${num(p.speed)} u/s; each touch takes ${pct(p.energy_fraction)} of their energy (${num(p.touch_interval_secs)} s apart, at most ${num(p.max_touches)} touches). It has ${num(p.hp)} hp; anyone can shoot it down, and it bursts into a scrap pile (${num(p.scrap_per_energy)} scrap per energy it took, at most ${num(p.max_scrap)}).`,
  gravity_coffee_break: (p) =>
    `Every gravity well takes a break (fading over ${num(p.fade_secs)} s): debris and ships drift free. Needs a level with wells.`,
  rubber_chickens: () => "Every new shot is a rubber chicken. Speed, damage and craters are unchanged; the squeak is not.",
  cat_paw: (p) =>
    `A giant paw reaches in through the boundary wall nearest the busiest group of ships and sweeps ${num(p.sweep_length)} u in ${num(p.sweep_secs)} s: ${num(p.damage)} damage and a ${num(p.knockback)} u/s shove to every ship it touches, debris swept along, three claw grooves raked into the rock (never in spawn zones). Watch for its shadow during the warning.`,
  tired_announcer: (p) =>
    `Every kill plays a ${num(p.killcam_secs)} s kill-cam for everyone, narrated by a very tired announcer.`,
  wrong_genre: () =>
    "The game becomes a side-scroller: every ship is locked to a flat plane facing left or right, shots fly in the plane, and hits ignore depth. Terrain stays 3D.",
  gossip_wall: () => "A face grows on the rock and gossips: every ship's position is shown to everyone, through walls and cloaks.",
  pizza_delivery: (p) =>
    `A delivery drone brings a pizza to a player (more likely the more damaged); hold still (under ${num(p.hold_speed)} u/s within ${num(p.hold_radius)} u) for ${num(p.hold_secs)} s in total and your hull is full again. Others can shoot the drone (${num(p.hp)} hp) and turn the pizza into a ${num(p.scrap)}-scrap pile for anyone.`,
  everything_is_cake: (p) =>
    `The level turns into cake: craters are ${num(p.crater_scale)}x as wide, and scavenging cake debris repairs ${num(p.scavenge_scale)}x its scrap in hull.`,
  duck_parade: (p) =>
    `${num(p.ducks)} giant rubber ducks drift in a convoy along a route through the level at ${num(p.speed)} u/s. They ignore rock, swallow shots and push ships aside (no damage). On asteroid belts they parade around the ring against the race.`,
  hat_rain: (p) =>
    `${num(p.hats_base)} hats (plus ${num(p.hats_per_player)} per player) drift down from high above and rest on rock: touch one to wear it. ${num(p.reveal_hats)} or more hats make you visible to everyone while it rains; hats scatter when you die. The results name the Hat Baron.`,
  customer_survey: (p) =>
    `A survey drone parks right in front of the leader's view and asks for a rating. It has ${num(p.hp)} hp: the star the destroying shot passes through is the leader's rating (anyone may answer).`,
  disco_inferno: (p) =>
    `The lights drop to ${pct(p.light_level)}, a mirror ball descends over the players and every ship leaves a light trail in its colour. Nothing else changes.`,
  big_head_day: (p) => `Every ship's hit sphere against shots is ${num(p.hit_scale)}x larger. Tunnels, ramming and pickups are unchanged.`,
  moving_day: (p) =>
    `Every pickup packs itself into a box and moves to another pickup point (they cannot be taken during the 2 s move; their timers are kept). Needs at least ${num(p.min_pickups)} pickups.`,
  mimic_pickups: (p) =>
    `Up to ${num(p.max_mimics)} weapon and upgrade pickups (3 plus one per 4 players) grow teeth: they hop at the nearest ship within ${num(p.range)} u at ${num(p.speed)} u/s and bite (${num(p.bite_damage)} damage, a shove). Each has ${num(p.hp)} hp; a killed mimic drops what it held. Needs at least ${num(p.min_pickups)} pickups.`,
  level_hiccups: (p) =>
    `Every ${num(p.interval_secs)} s (give or take ${num(p.jitter_secs)}) the level hiccups: ships, debris and loose mines hop about ${num(p.lift)} u along "up", shots in flight are nudged.`,
};

/** Rule set ids by event bans (`events: { "<id>": Banned }`). */
function bannedEvents(def) {
  const ev = def.events;
  if (!(ev instanceof Map)) return [];
  return [...ev.entries()].filter(([, v]) => tagOf(v) === "Banned").map(([k]) => k);
}

function eventsDoc(events, director, rulesets) {
  const d = director?.def ?? {};
  const list = events.filter((e) => e.id !== "director");
  const nameOf = new Map(list.map((e) => [e.id, e.def.name ?? e.id]));
  const out = [
    header(
      "Comedy event reference",
      "data/events/*.ron and data/rulesets/*.ron",
      [
        "The level is a participant: a comedy director rolls events during a match, weighted by the",
        "world's **Absurdity** slider and capped by the rule set. How the events feel is in the",
        "[player manual](../player-manual.md#comedy-events); this page lists the numbers. A server's own",
        "rule set can lower the cap (0 = no events) and ban single events.",
      ].join("\n"),
    ),
    "## When events happen",
    "",
    table(
      ["Rule", "Value"],
      [
        ["Effective Absurdity", "the lower of the world's Absurdity slider and the rule set's cap"],
        ["Warning before every event", `${num(d.announce_secs)} s: banner, announcer line and a tell in the world`],
        ["Rolls", `every ${num(d.roll_secs)} s, chance ${pct(d.chance_at_100)} x Absurdity / 100 per roll`],
        ["Quiet times", `no event in the first ${num(d.quiet_start_secs)} s of a match, its last ${num(d.quiet_end_secs)} s, the warm-up, the countdown or the results`],
        ["Between events", `at least ${num(d.gap_secs)} s from one event's end to the next warning (${num(d.calm_gap_scale)}x in a calm mood); one event at a time, no repeats while others can run`],
        ["Moods", `the level is calm, bored, grumpy or giddy, changing every ${num(d.mood_min_secs)}-${num(d.mood_max_secs)} s; a mood makes its events ${num(d.mood_bias)}x as likely`],
        ["Damage from events", "never gives kill credit (the killer is \"the level\")"],
      ],
    ),
    "",
    "About 1.2 events per 5-minute match at Absurdity 20, 3 at 50 and 4.5 at 100.",
    "",
    "## The events",
    "",
    table(
      ["Event (id)", "Lasts", "Mood", "Weight", "What happens"],
      list.map((e) => {
        const def = e.def;
        const p = def.params instanceof Map ? Object.fromEntries(def.params) : {};
        const text = EVENT_TEXT[e.id]?.(p, def) ?? "";
        const lasts = `${num(def.active_secs)} s${def.ending_secs ? ` (+${num(def.ending_secs)} s)` : ""}`;
        const moods = (def.moods ?? []).map((m) => MOOD_NAMES[tagOf(m)] ?? tagOf(m)).join(", ") || "any";
        const name = `**${def.name ?? e.id}** (\`${e.id}\`)${def.enabled === false ? " (disabled)" : ""}`;
        return [name, lasts, moods, num(def.weight), text];
      }),
    ),
    "",
    "Lasts: after the warning; (+) is the wind-down (drones leave, the face sinks). Mood: the mood that",
    "doubles the event's chance. Weight: its share of the draw among the events that can run.",
    "",
    "## Modes and rule sets",
    "",
    "Some modes leave events out where they would break the mode: **The Architect** runs no director",
    "(the architect's Absurdity stroke starts events instead); **The Squeeze** has no Cat paw or Duck",
    "parade once the walls are in phase 4; **Belt Up** has no Bored level, Wrong genre or Moving day,",
    "and no event during start lights, a heat's first 15 s or heat results; the **Instagib** mutator",
    "has no Tax drone or Pizza delivery (energy and hull are hidden).",
    "",
    table(
      ["Rule set (id)", "Absurdity cap", "Banned events"],
      rulesets.map((r) => {
        const bans = bannedEvents(r.def).map((id) => nameOf.get(id) ?? id);
        return [`**${r.def.name ?? r.id}** (\`${r.id}\`)`, num(r.def.absurdity ?? 20), bans.join(", ") || "–"];
      }),
    ),
    "",
  ];
  return out.join("\n");
}

// ---------------------------------------------------------------------------

export function generate() {
  const weapons = loadDir("weapons");
  const ships = loadDir("ships");
  const gadgets = loadDir("gadgets");
  const rulesets = loadDir("rulesets");
  const rulesPath = join(dataDir, "rulesets", "paint_the_void.ron");
  const rules = existsSync(rulesPath) ? parseRon(readFileSync(rulesPath, "utf8"), rulesPath) : null;
  const catalogPath = join(repoRoot, "web", "src", "rules", "catalog.ts");
  const catalog = existsSync(catalogPath) ? catalogModes(readFileSync(catalogPath, "utf8")) : new Map();
  const tinker = ships.find((s) => s.id === "tinker")?.def;
  const events = loadDir("events");
  const director = events.find((e) => e.id === "director");
  return {
    "weapons.md": weaponsDoc(weapons, rules, tinker?.hull_max ?? 100),
    "ships.md": shipsDoc(ships),
    "gadgets.md": gadgetsDoc(gadgets, weapons, rules),
    "modes.md": modesDoc(rulesets, weapons, gadgets, catalog),
    "events.md": eventsDoc(events, director, rulesets),
  };
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    console.log("usage: node tools/gen-weapon-docs.mjs [--check]");
    return;
  }
  const unknown = args.filter((a) => a !== "--check");
  if (unknown.length) {
    console.error(`gen-weapon-docs: unknown argument ${unknown[0]}`);
    process.exit(2);
  }
  const check = args.includes("--check");
  const files = generate();
  let stale = 0;
  for (const [name, text] of Object.entries(files)) {
    const path = join(outDir, name);
    const rel = relative(repoRoot, path).replace(/\\/g, "/");
    const current = existsSync(path) ? readFileSync(path, "utf8").replace(/\r\n/g, "\n") : null;
    if (current === text) {
      if (check) console.log(`up to date: ${rel}`);
      continue;
    }
    if (check) {
      console.error(`stale: ${rel} (${current === null ? "missing" : "differs from data/"})`);
      stale++;
    } else {
      mkdirSync(outDir, { recursive: true });
      writeFileSync(path, text);
      console.log(`wrote ${rel}`);
    }
  }
  if (stale) {
    console.error("gen-weapon-docs: run `node tools/gen-weapon-docs.mjs` and commit the result");
    process.exit(1);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
