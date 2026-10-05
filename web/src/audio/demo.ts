// `?audio=demo` (M4 A6): the sound check page. Lists every registered sound
// by group with a play button (loops toggle), plays them centred or from a
// chosen direction and distance through the real engine and mixer, runs a
// 64-sound burst against the voice cap, and lets each announcer voice pack
// say any of its lines with the in-game caption. Used for listening reviews
// and by the browser smoke test (`window.__lsAudioDemo`).

import type { Vec3 } from "../math/vec3.ts";
import { Announcer, portraitSvg } from "../ui/announcer.ts";
import { ALL_TRIGGERS, type Trigger } from "./announcer/lines.ts";
import { PACKS } from "./announcer/packs.ts";
import { defDuration } from "./dsp.ts";
import { VOICE_CAP, type LoopHandle } from "./engine.ts";
import { audioEngine } from "./index.ts";
import { allSounds, type SoundDef } from "./library.ts";
import { audioSettingsHtml, bindAudioSettings } from "./settings.ts";

export function wantsAudioDemo(loc: Location): boolean {
  return new URLSearchParams(loc.search).get("audio") === "demo";
}

/** Where the demo plays positional sounds from (the listener sits at the origin facing -z). */
const PLACES: readonly { id: string; label: string; at: Vec3 | null }[] = [
  { id: "centre", label: "Centred", at: null },
  { id: "left", label: "Left 10 u", at: { x: -10, y: 0, z: 0 } },
  { id: "right", label: "Right 10 u", at: { x: 10, y: 0, z: 0 } },
  { id: "behind", label: "Behind", at: { x: 0, y: 0, z: 12 } },
  { id: "above", label: "Above", at: { x: 0, y: 12, z: -4 } },
  { id: "far", label: "Far (120 u)", at: { x: 30, y: 10, z: -116 } },
];

/** Quick lines per pack card. */
const QUICK: readonly { trigger: Trigger; label: string }[] = [
  { trigger: "firstBlood", label: "First blood" },
  { trigger: "multiKill3", label: "Triple kill" },
  { trigger: "event.duck_parade", label: "Duck parade" },
  { trigger: "caveIn", label: "Cave-in" },
  { trigger: "killcam", label: "Kill-cam" },
  { trigger: "beltUp", label: "Belt up" },
];

const VARS = { killer: "Rivet", victim: "Sprocket", target: "Gasket", winner: "Torque" };

function esc(s: string): string {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c);
}

function soundRow(d: SoundDef): string {
  const secs = defDuration(d.synth);
  return `
      <li class="snd-row">
        <button type="button" class="snd-play${d.loop ? " loop" : ""}" data-sound="${esc(d.id)}" aria-label="Play ${esc(d.label)}">
          <span class="snd-icon" aria-hidden="true"></span>
        </button>
        <div class="snd-text">
          <div class="snd-name">${esc(d.label)} <code>${esc(d.id)}</code></div>
          <div class="snd-desc">${esc(d.description)}</div>
        </div>
        <span class="snd-len">${d.loop ? "loop" : `${secs.toFixed(secs < 1 ? 2 : 1)} s`}</span>
      </li>`;
}

export function showAudioDemo(ui: HTMLElement): void {
  const engine = audioEngine();
  const sounds = allSounds();
  const groups = new Map<string, SoundDef[]>();
  for (const d of sounds) groups.set(d.group, [...(groups.get(d.group) ?? []), d]);
  document.body.classList.add("audio-demo-page");
  const root = document.createElement("div");
  root.className = "audio-demo";
  const packCards = PACKS.map(
    (p) => `
        <article class="ad-pack" data-pack="${p.id}" style="--ann-accent:${p.accent}">
          <div class="ad-pack-head">
            <div class="ad-pack-portrait">${portraitSvg(p)}</div>
            <div><div class="ad-pack-name">${esc(p.name)}</div><div class="ad-pack-title">${esc(p.title)}</div></div>
          </div>
          <div class="ad-pack-quick">${QUICK.map((qk) => `<button type="button" class="ad-chip" data-say="${qk.trigger}">${esc(qk.label)}</button>`).join("")}</div>
          <div class="ad-pack-any">
            <select aria-label="Line">${ALL_TRIGGERS.map((t) => `<option value="${t}">${t}</option>`).join("")}</select>
            <button type="button" class="ad-chip strong" data-say-select>Say</button>
          </div>
        </article>`,
  ).join("");
  const groupHtml = [...groups.entries()]
    .map(
      ([g, ds]) => `
      <section class="ad-group">
        <h2>${esc(g)} <span>${ds.length}</span></h2>
        <ul>${ds.map(soundRow).join("")}</ul>
      </section>`,
    )
    .join("");
  root.innerHTML = `
    <header class="ad-header">
      <div>
        <h1>Sound check</h1>
        <p>Every sound of Loose Screws, synthesized in your browser. No recordings.</p>
      </div>
      <div class="ad-status">
        <span class="ad-pill" data-state="locked"><i></i><b class="ad-state">Click anywhere to switch the sound on</b></span>
        <span class="ad-meter"><span class="ad-meter-label">Voices</span><span class="ad-meter-bar"><span class="ad-meter-fill"></span></span><b class="ad-voices">0 / ${VOICE_CAP}</b></span>
      </div>
    </header>
    <div class="ad-layout">
      <aside class="ad-side">
        <form class="ad-card ad-mixer" novalidate>${audioSettingsHtml()}</form>
        <div class="ad-card">
          <h3>Play from</h3>
          <div class="ad-places">${PLACES.map((p, i) => `<label class="ad-place"><input type="radio" name="place" value="${p.id}"${i === 0 ? " checked" : ""}/><span>${p.label}</span></label>`).join("")}</div>
          <p class="ad-note">You sit at the centre, facing forward. Headphones show the direction best.</p>
        </div>
        <div class="ad-card">
          <h3>Voice cap</h3>
          <button type="button" class="ad-chip strong" data-burst>Fire a 64-sound burst</button>
          <p class="ad-note ad-burst-result">At most ${VOICE_CAP} voices play at once; the quietest and least important make room.</p>
        </div>
      </aside>
      <main class="ad-main">
        <section class="ad-group ad-announcers">
          <h2>Announcer voice packs <span>${PACKS.length}</span></h2>
          <div class="ad-packs">${packCards}</div>
        </section>
        ${groupHtml}
      </main>
    </div>`;
  ui.appendChild(root);
  const announcer = new Announcer(ui, engine, PACKS[0] as (typeof PACKS)[number]);
  bindAudioSettings(root.querySelector(".ad-mixer") as HTMLFormElement, () => engine.play("caption.blip"));
  engine.setListener({ x: 0, y: 0, z: 0 }, { x: 0, y: 0, z: -1 }, { x: 0, y: 1, z: 0 });

  const place = (): Vec3 | null => {
    const v = (root.querySelector('input[name="place"]:checked') as HTMLInputElement | null)?.value;
    return PLACES.find((p) => p.id === v)?.at ?? null;
  };
  const loops = new Map<string, LoopHandle>();
  root.addEventListener("click", (ev) => {
    engine.unlock();
    const t = ev.target as HTMLElement;
    const btn = t.closest<HTMLButtonElement>("button");
    if (!btn) return;
    const id = btn.dataset["sound"];
    if (id) {
      const def = sounds.find((d) => d.id === id);
      if (def?.loop) {
        const h = loops.get(id);
        if (h?.playing) {
          h.stop(0.3);
          loops.delete(id);
          btn.classList.remove("playing");
        } else {
          const nh = engine.loop(id, { position: place() });
          if (nh) {
            loops.set(id, nh);
            btn.classList.add("playing");
          }
        }
      } else {
        engine.play(id, { position: place() });
        btn.classList.remove("pulse");
        void btn.offsetWidth;
        btn.classList.add("pulse");
      }
      return;
    }
    const card = btn.closest<HTMLElement>(".ad-pack");
    if (card) {
      const pack = PACKS.find((p) => p.id === card.dataset["pack"]);
      if (!pack) return;
      const trigger = (btn.dataset["say"] ?? (btn.hasAttribute("data-say-select") ? card.querySelector("select")?.value : undefined)) as Trigger | undefined;
      if (!trigger) return;
      if (announcer.voicePack.id !== pack.id) {
        announcer.clear();
        announcer.setPack(pack);
      }
      announcer.say(trigger, VARS);
      return;
    }
    if (btn.hasAttribute("data-burst")) {
      const before = engine.stats();
      const ids = sounds.filter((d) => !d.loop && d.bus === "effects").map((d) => d.id);
      let started = 0;
      for (let i = 0; i < 64; i++) {
        const a = (i / 64) * Math.PI * 2;
        if (engine.play(ids[i % ids.length] as string, { position: { x: Math.cos(a) * 14, y: (i % 5) - 2, z: Math.sin(a) * 14 }, gain: 0.5 })) started++;
      }
      const s = engine.stats();
      const res = root.querySelector(".ad-burst-result") as HTMLElement;
      res.textContent = `${started} of 64 started; ${s.voices} voices now (cap ${VOICE_CAP}); ${s.stolen - before.stolen} replaced, ${s.dropped - before.dropped} dropped.`;
      (window as unknown as { __lsAudioDemo?: { burst?: unknown } }).__lsAudioDemo!.burst = { started, voices: s.voices, peak: s.peakVoices };
    }
  });

  const pill = root.querySelector(".ad-pill") as HTMLElement;
  const stateText = root.querySelector(".ad-state") as HTMLElement;
  const voices = root.querySelector(".ad-voices") as HTMLElement;
  const fill = root.querySelector(".ad-meter-fill") as HTMLElement;
  const tick = (): void => {
    const s = engine.stats();
    pill.dataset["state"] = s.state;
    stateText.textContent = s.state === "locked" ? "Click anywhere to switch the sound on" : s.state === "running" ? `Sound on · ${s.rendered}/${s.total} sounds ready` : `Sound ${s.state}`;
    voices.textContent = `${s.voices} / ${VOICE_CAP}`;
    fill.style.width = `${(100 * s.voices) / VOICE_CAP}%`;
    for (const [id, h] of loops) {
      if (!h.playing) {
        loops.delete(id);
        root.querySelector(`[data-sound="${id}"]`)?.classList.remove("playing");
      }
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  document.addEventListener("pointerdown", () => engine.unlock(), { once: true });
  (window as unknown as { __lsAudioDemo?: unknown }).__lsAudioDemo = {
    expected: sounds.length,
    listed: root.querySelectorAll("[data-sound]").length,
    packs: PACKS.length,
    said: () => [...announcer.said],
  };
}
