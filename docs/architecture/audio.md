# Audio and the announcer (M4 A6)

The browser client's sound: a WebAudio engine with procedurally synthesized sounds (no
recorded assets), positional mixing, volume settings, and the announcer, a caption character
with a synthesized gibberish voice in three voice packs. Plan: [milestone-4-plan.md
§3.7](../milestone-4-plan.md#37-client-workshop-audio-announcer). Everything is client-side and
cosmetic: nothing here is hashed or replicated.

| File (`web/src/`) | Contents |
| --- | --- |
| `audio/dsp.ts` | Synthesis: a `SynthDef` is a list of layers (sine, triangle, saw and square with polyBLEP, white, pink and brown noise, crackle, Karplus-Strong pluck, modal metal), each with pitch sweep, vibrato, FM, envelope, two filters (RBJ biquads, swept), tremolo, drive and repeats; seeded RNG per variant; seamless loops by crossfade; the reverb's impulse response |
| `audio/library.ts` | `SoundDef` (synth plus mix: bus, gain, priority, instance limit, minimum gap, distance, reverb send, pitch jitter, loop) and the registry (`registerSounds`, `soundDef`, `allSounds`) |
| `audio/sounds/*.ts` | The sound set, one file per group (`weapons.ts`, `impacts.ts`, `mechanics.ts`, `announcer.ts`, `world.ts`: M4 B5's mood drones, well hum, cave-in and pigment), listed in `sounds/index.ts` |
| `audio/engine.ts` | `AudioEngine`: gesture unlock, buses and mixer, voice cap, positional voices, loops, background pre-rendering, listener, settings, stats |
| `audio/settings.ts` | Volumes (master, effects, music, announcer, 0-100) and mute in `localStorage` `ls.audio`; the start screen's Sound fieldset |
| `audio/game.ts` | `GameAudio`: turns each frame (projectiles, ships, cues, pickups, boost) into sounds and kills into announcer triggers |
| `audio/voice.ts` | The gibberish formant voice: `renderSpeech(line, voice, rate, seed)` returns samples and each word's time span |
| `audio/announcer/` | Voice packs (`tired.ts` Doug, `overexcited.ts` Sparky, `passiveAggressive.ts` Brenda), triggers and line picking (`lines.ts`), `packForSeed` (`packs.ts`) |
| `ui/announcer.ts`, `ui/announcer.css` | The caption (portrait, name, line revealed word by word, mouth moving) and its queue |
| `audio/index.ts` | `audioEngine()` (the page's engine), `MatchAudio` (what `main.ts` runs per game), `window.__lsAudio` |
| `audio/demo.ts`, `audio/audio.css` | `?audio=demo`, the sound check page |

## Engine

- **No AudioContext before a user gesture.** The engine listens for `pointerdown`, `mousedown`,
  `keydown`, `touchend` and `click` (capture) and creates the context in the first one; when the
  page already had one (`navigator.userActivation.hasBeenActive`, e.g. the Play click before the
  game builds its `MatchAudio`) it starts at once. `play` before that does nothing. The browser
  smoke test checks this with `tools/browser-smoke/audio-probe.mjs`.
- **Rendering.** A sound is rendered into an `AudioBuffer` the first time it plays; right after the
  context starts, every registered sound is rendered in the background in 6 ms slices (first one
  variant of each, then the rest; 3 variants by default, 1 for loops), so by the time a match
  loads everything is ready. The whole set (45 sounds, about 115 buffers) takes about 0.5 s of CPU
  at 48 kHz; the largest single sound (the Tectonic rumble loop) about 30 ms.
- **Mixer.** voice gain -> (positional: distance low-pass -> `PannerNode` HRTF, inverse distance
  with the sound's `refDistance`) -> bus (effects, music, voice) -> master -> limiter
  (`DynamicsCompressor`, -6 dB, 12:1) -> speakers. The effects bus passes a ducker the announcer
  pulls to 55 % while it speaks. Each voice also sends to one convolver reverb (2.2 s), more for
  distant sounds. Volumes use a squared curve (slider 50 = 25 % gain).
- **Voice cap 32.** A new sound beyond the cap replaces the least important voice (priority +
  4 x estimated audibility - 0.8 per second of age) or is dropped if it matters less. Per sound:
  `maxInstances` (the oldest instance makes room) and `minGap` (separately for centred and
  positional plays). Positional sounds estimated quieter than 0.4 % are not started.
  `stats()` counts started, stolen, dropped and culled voices.
- **Listener.** `setListener(position, forward, up)` once per frame from the camera; sounds with a
  `position` (world units) are positional, the rest (own ship, UI, announcer) play centred.
- **Loops.** `loop(id)` returns a handle (`setGain`, `setRate`, `setPosition`, `stop`); a loop
  can be stolen like any voice (`playing` turns false). `setGain(g)` sets the loop to `g` times
  the sound's mix gain: it replaces the start's `gain` option, so a loop started silent (the
  boost) can be faded up. A positional loop that starts below `CULL_GAIN` is not started at
  all, so start loops at the gain they should have (M4 D2).
- The context suspends while the tab is hidden.

## Game sounds (`GameAudio`)

| Source | Sound |
| --- | --- |
| New projectile near its owner (matched frame to frame like `game/vfx.ts`) | `fire.<shape>`: bolt, orb, arc, pellet, harpoon, rocket (missile: rocket played 18 % higher), lob (mine, grenade), paint, sneeze (blob); bigger visual scale plays lower (`rate = scale^-0.35`); own shots centred |
| Projectile gone, not at a ship | by the shape's impact kind (`VFX_SHAPES`): `impact.sparks`, `impact.plasma`, `impact.splat`, `impact.thud`, `explosion.small` / `explosion.big` (blast radius 3.5 u and more) |
| Hit cue (`CUE_HIT`) | `impact.hull`, `impact.shield` (target shielded), `impact.hullOwn` (you) |
| A ship's `alive` drops | `explosion.ship` (yours centred, with `ship.powerdown`) |
| Your respawn | `ship.respawn` |
| Your boost (held and boost left) | `boost.loop` (gain and pitch follow a smoothed level), `boost.start` |
| Another ship starts boosting | `boost.start` at the ship |
| A pickup vanishes within 5 u of your ship | `pickup` |
| Cues | Polite warning `polite.chime` (louder when aimed at you), Tectonic launch `tectonic.launch`, armed `tectonic.rumble` loop swelling until the blast (`tectonic.blast` on its shockwave), other shockwaves `explosion.big`, Railspike muzzle flash `fire.spike`, charge start `charge.start`, overcharge `overcharge`, Leech Arc beam `beam.leech`, swap `swap`, rewind `rewind`, Flash Pod `flash.bang` (+ `flash.ring` when you are blinded), gadget on `gadget.shield` / `gadget.cloak` / `gadget.generic`, decoy return `decoy.return`, paint colour `paint.select` |

World sounds (M4 B5, `game/worldFx.ts` `WorldAudio`, `game/moodFx.ts`): `cavein.warning` at the
cracking ceiling and `cavein.crash` when it comes down (plus the announcer's `caveIn` within 40 u
of your ship), `pigment.splash` where pigment is taken, the nearest gravity well's `well.hum`
loop within 16 u (swelling closer), and one ambient drone per mood (`mood.calm`, `mood.bored`,
`mood.grumpy`, `mood.giddy`, music bus) crossfaded over the mood's 6 s ease and faded out on the
results screen and in the lobby.

Nothing farther than 240 u from the camera plays. At the results screen the effects stop.

## Announcer

`MatchAudio.announcer.say(trigger, vars)` picks a line of the trigger from the match's voice pack
(never the same line twice in a row), fills `{killer}`, `{victim}`, `{target}`, `{winner}`,
plays the trigger's sting, renders the speech (24 kHz, a few ms), reveals each word as it is
spoken, ducks the effects and hides the caption 1.6 s after the voice ends. At most two lines
wait; an event announcement jumps the queue. Captions show without sound too.

Triggers: `event.<id>` for the 18 events (ids in `EVENT_IDS`, the ls-sim `EventKind` order),
`firstBlood`, `multiKill2`, `multiKill3`, `multiKill4` (kills within 4 s by one pilot),
`killcam` (the Tired announcer event), `caveIn`, `matchStart` (after the countdown; at once in
practice), `matchEnd`, `beltUp`, `generic`. Today the client says `matchStart`, `firstBlood` and
the multi-kills (a death credits the last attacker that hit the ship within 3 s); the event,
cave-in, kill-cam and Belt Up packages call `say` with theirs.

The pack is `packForSeed(hashSeed(shareCode))` until the match seed reaches the client. Voice
parameters (`VoiceParams`): base pitch, pitch range, syllables per second, declination, the last
word's melody (fall, rise, singsong, flat), breathiness, formant scale, vibrato, pause length,
brightness, emphasis for CAPS and "!" words, glide. Each vowel group of a word is a syllable whose
vowel colour comes from its first vowel letter and whose onset is a consonant sound of the letter
before it (plosive burst, sibilant, hush, fricative, aspirate, nasal, glide); a word's pitch
pattern is seeded by the word, so a line always sounds the same in one pack.

## Adding sounds

1. Add a `SoundDef` to the group file in `web/src/audio/sounds/` (or a new file listed in
   `sounds/index.ts`); ids are `group.name`. Loops need `loop: true` and `synth.loopFade`.
2. Play it: `audioEngine().play(id, { position })` or `.loop(id, ...)`.
3. Check it: `?audio=demo` (every registered sound has a play button; the smoke test fails when
   one is missing) and `node web/scripts/audio-preview.ts --only <id>` (WAV, spectrogram PNG and
   levels in `target/audio-preview`). `src/audio/audio.test.ts` renders every sound (finite, not
   silent, normalized, right length).

Mix guide: one-shots are normalized to a 0.9 peak, `gain` sets the mix (fire 0.5-0.75, impacts
0.4-0.55, explosions 0.85-1); priority 3-4 for frequent small sounds, 7-10 for what must not be
lost (blasts, deaths, the Polite chime, the boost loop, the announcer).
