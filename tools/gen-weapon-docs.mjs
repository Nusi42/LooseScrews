#!/usr/bin/env node
// Generates the player reference tables in userDocs/reference/ from the game data:
//   data/weapons/*.ron  -> userDocs/reference/weapons.md
//   data/ships/*.ron    -> userDocs/reference/ships.md
//   data/gadgets/*.ron  -> userDocs/reference/gadgets.md (with the Shield setting per weapon)
// The default rule set (data/rulesets/paint_the_void.ron) supplies the weapon order (the
// design doc's) and the "default rules" columns.
//
// Usage: node tools/gen-weapon-docs.mjs [--check]
//   (no flag)  write the three files
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

export function generate() {
  const weapons = loadDir("weapons");
  const ships = loadDir("ships");
  const gadgets = loadDir("gadgets");
  const rulesPath = join(dataDir, "rulesets", "paint_the_void.ron");
  const rules = existsSync(rulesPath) ? parseRon(readFileSync(rulesPath, "utf8"), rulesPath) : null;
  const tinker = ships.find((s) => s.id === "tinker")?.def;
  return {
    "weapons.md": weaponsDoc(weapons, rules, tinker?.hull_max ?? 100),
    "ships.md": shipsDoc(ships),
    "gadgets.md": gadgetsDoc(gadgets, weapons, rules),
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
