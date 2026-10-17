# Milestone 5 — Platforms and ops

Status: **decisions taken** (§8), ready to start after the code-quality audit's fix wave ([code-quality-audit.md](code-quality-audit.md) §4) · Design source: [design/game-design-doc.md](design/game-design-doc.md) (Tech architecture: Rendering, Networking, Bandwidth for 32 players, Servers; Monetization for the desktop app) · Previous milestone: [milestone-4-plan.md](milestone-4-plan.md) · Overall plan: [implementation-plan.md](implementation-plan.md)

Milestone 5 takes the game from "runs on the developer's machine in Chrome" to "runs on more machines, and other people can run it". The design doc calls for a Tauri desktop app with a UDP transport, a WebRTC fallback for browsers without WebTransport, a WebGL2 fallback for browsers and GPUs without WebGPU, regional server deployment through an orchestrator (Agones or Nomad), and bandwidth optimization for 32-player matches. M5 builds:
- a Tauri desktop app with a UDP transport (QUIC datagrams);
- regional deployment artifacts for Nomad, ops endpoints and a local multi-region simulation;
- bandwidth optimization for 32-player matches.

Decision: the WebRTC fallback and the WebGL2 renderer are dropped. Every supported browser has what the game needs: WebGPU, and WebTransport with the existing WebSocket fallback. Browsers without WebGPU are not supported and get a clear message. Decision: the real cloud deployment is not part of M5; hosting is set up separately, later.

M5 also takes over every carried-over item that [implementation-plan.md §8](implementation-plan.md#carried-over-items) schedules into M5: the bandwidth worst case, the exact lag-compensation view tick, hosted CI (declined), wall-clock tick timing, the Chrome `powerPreference` warning, reference mid-GPU timings (measured by the maintainer on real hardware, outside the M5 packages), and the kill-cam after your own death. It also takes the security and robustness gaps found in M3, and the M4 exceptions and leftovers.

Requirements taken before planning:
- The standing bar for everything visual: the first impression must be strong. This applies to the desktop app as much as to new effects. Every package that draws something adds gallery scenes and sends before/after shots for the phone review.
- Browser automation stays headless and injects `tools/browser-smoke/fake-pointer-lock.mjs` on every page, scratch scripts included. That covers the gallery, the smoke and anything that drives the desktop app's web view.
- M4 closed with the own-death kill-cam moved here under M4's cut order, and the grumpy mood's extra cave-ins not yet built. Decision: both are first-wave items (question 11).

## 1. Scope

| Area | In scope |
| --- | --- |
| Desktop app | A Tauri 2 app for Windows x64 only (question 3). It runs the same web client and WASM core in the system web view (question 5), with a native network bridge in its Rust backend (question 4). It bundles the client, so offline practice works. It also has a server list without browser restrictions, app-local identity, fullscreen and pointer lock, and a per-user installer. M5 ships unsigned tester builds (question 6) |
| Native transport for the app | The app's Rust side talks to `ls-server` over UDP: QUIC datagrams through the existing WebTransport endpoint (question 4). The native WebSocket and WebTransport clients now in `ls-bots` move into a shared crate (`crates/ls-netio`), which the bots and the app both use |
| Browser support | WebGPU only (question 7), with WebTransport and the existing WebSocket fallback. Browsers and GPUs without WebGPU get a clear message that names the requirement and the supported browsers (§3.2). No WebRTC fallback and no WebGL2 renderer |
| Regional deployment | Deployable artifacts: Linux release builds of `ls-server`, `ls-directory` and the web client, Nomad job specs (question 2), a Caddy TLS front, systemd units as a fallback. `ls-server` ops endpoints: health, metrics, graceful drain, JSON logs. Regions in the directory and a **Quick play** that picks the best region by ping. A **local multi-region simulation** (`tools/fleet.mjs`) and a local Nomad dev agent stand in for the real deployment (question 1) |
| Bandwidth | Protocol 7: new projectiles predicted from their owner's orientation and the weapon's muzzle speed, which closes the M2 worst case. A first live 32-player room (31 bots and a probe): per-client bandwidth and tick time measured, and the join (`Welcome` with edit log) size recorded |
| Carried over (from M2) | **Bandwidth worst case**: the busiest snapshots are up to about 6 % over budget at a 4-tick baseline and 19 % at 6 ticks (`busy_fight_with_projectiles_fits_the_budget`). New projectiles' velocity residuals dominate |
| Carried over (from M1) | **Exact lag-compensation view tick**: the server estimates what the shooter saw from the acked snapshot. Sending the client's render tick in `Input` makes it exact (protocol change) |
| Carried over (from M1/M2) | **Hosted CI**: declined by the maintainer (question 9). Verification stays local: `tools/ci.*`, run by the pre-push hook on every push to the public remote |
| Carried over (from M2) | **Tick timing is wall-clock**: e2e and benchmark p99 checks fail under outside load. M5 measures the sim thread's CPU time per tick beside wall time, and asserts on CPU time |
| Carried over (from M2) | **Chrome/Edge console warning** "powerPreference option is currently ignored" on Windows (crbug 369219127): drop the option on Windows |
| Carried over (from M3, via M4) | **Reference mid-GPU timings**: the maintainer measures on real hardware separately (question 10); M5 rents no GPU. M5 re-measures the integrated-GPU floor (B6) |
| Carried over (from M4, first wave) | **Kill-cam after your own death**: a setting, on by default in modes with respawns, skipped by any key (M4 §3.7, question 13 there). The recorder, the clip timeline and the picture-in-picture window already exist for the Tired announcer (`web/src/game/killcam.ts`) |
| M4 not built (first wave) | **The grumpy mood's extra cave-ins** (M4 §3.1 "grumpy: ... more cave-ins"): while the level is grumpy, the cave-in strain threshold drops (§3.7) |
| M3 security gaps (D1) | Duel answers and party tags work without a token. Directory address squatting and fake presence registrations are possible. Failing next-room builds get no backoff. A panicked private room thread keeps its slot. A join racing a closing private room gets no `Reject` |
| M3 gaps (D3) | Private rooms and duels ignore `--ban` and `--balance-list`. On `https://` pages the server hands browsers its plain-HTTP `--directory` URL. The directory does no forwarded-for handling behind a proxy |
| M3 web items (D2) | The party panel's polls never stop after a disconnect. The 7-column server table scrolls sideways at phone width. The weapon panel overlaps the hull panel at 360 px. The lobby card is not focused when it appears |
| M4 exceptions | **Bot input stalls**: all 8 bots stall 57-250 ms per think turn at once on a quiet machine, mostly in Architect matches (6 input-starvation warnings in the M4 soak); the bots also start sending inputs for a new room only after its first snapshot. **The 1.2-2 u rock blade in `cave-sunbeam`** still shows steps. **Mid-GPU timings** are only estimated: the integrated-GPU floor needs about 60 ms for the sunlit-cave frames, and the lever there is cheaper far terrain. **No cave-in remesh benchmark** exists |
| M4 web leftovers (D2) | Race portal hum and Architect ink particles keep going on the results screen. The mood badge slightly overlaps the mode bar on phones. iOS audio resumes only after the page is hidden and shown again. `ArchitectHud` rebuilds its plate on every change |

Out of scope (M6 or later, or outside the milestones): coop modes, cosmetics store and creator economy (M6). The WebRTC fallback and the WebGL2 renderer (dropped). The real cloud deployment, which the maintainer sets up separately, later (question 1). Code signing, the app updater and its keys, and store publishing, which the maintainer sets up separately, close to the finished product (question 6). Measuring the reference mid-range GPU, done by the maintainer on real hardware (question 10). Accounts and persistent friends lists (question 8). macOS and Linux desktop builds (question 3). Autoscaling and matchmaking by skill; ranked playlists with the live balance list. Touch controls and gamepads. Team Brawl on belt worlds (M4 §8.1.8 named it a candidate). The weak bot architect and rare bumps between racers (M4 bots). The remaining M4 D1 items (a heat reset's event end only with the next snapshot; joining during start lights resets the heat; the lag-compensation overlap check runs in every mode; more than 8 Belt Up players), unless a package owns the file anyway. The pending M4 visual and listening sign-off is a separate review, not M5 work.

## 2. Current state (start of M5)

- **Transports.** Browsers connect over WebTransport (UDP, `wtransport` 0.7, self-signed ECDSA certificate trusted by hash from `GET /cert-hash`, 13 days, rotated every 6) with a WebSocket fallback (`web/src/net/connect.ts`, `transport.ts`, `webtransport.ts`; `?transport=ws|wt`). Both feed one `Hub`, so rooms never know the transport. There is no WebRTC. `ls-bots` has native WebSocket and WebTransport clients (`crates/ls-bots/src/transport.rs`), the only native client code in the repo.
- **Secure context.** WebGPU *and* WebTransport need a secure context. A page opened as `http://<LAN-IP>:7777` shows the "needs WebGPU" error screen (`web/src/ui/errorScreen.ts`), not only a WebSocket fallback. Public hosting therefore needs HTTPS with a real domain: the hosting guide documents a Caddy reverse proxy.
- **Renderer.** WebGPU only, about 19,000 lines in `web/src/renderer/` (TS including the shader builders; about 4,200 lines of WGSL builders in `shaders/`). It has two compute pipelines (the light froxel masks in `lights.ts` and the rock texture generation in `rockTextures.ts`) and storage buffers read in fragment shaders (the light masks). It renders into `rgba16float` with MSAA 4x, uses `timestamp-query` when present, and selects quality presets high/medium/low automatically. `initGpu` (`gpu.ts`) throws `GpuUnavailableError`, and there is no other backend. Gallery: 173 scenes. Integrated-GPU floor (M4 D5, Radeon 2 CU, 1080p high): median 24.7 ms, p90 60.2 ms, sunlit caves about 60 ms.
- **Server.** `ls-server` serves static files, `/ws`, `/cert-hash` and the `/api/...` JSON API on one TCP port, and WebTransport on UDP. It has no health or metrics endpoints (only `tick stats` log lines), handles only Ctrl+C (no drain), and has no TLS for HTTP. `ls-directory` is plain HTTP, in-memory, with per-IP rate limits but no forwarded-for handling. Tick times are wall clock (`stats.rs`).
- **Machine and repo.** Development and every recorded run are on Windows 11 (Ryzen 9 9950X3D, shared). There is no Docker, no WSL distribution, no Nomad, no `gh`; the Rust targets installed are the Windows ones and `wasm32-unknown-unknown`. The repo has no hosted remote: `origin` of the work folders is the local master.
- **Bandwidth.** 32 busy ships with about 100 projectiles in flight: whole deltas average 392-405 / 476-491 / 544-560 B against 2 / 4 / 6-tick baselines (budget 576 B per snapshot). The busiest reach 471-482 / 565-612 / 614-683 B. New projectiles are about 10 B each, mostly a 43-bit velocity residual and a 28-bit position residual. No live room has had more than 8 bots (plus Welcome probes).
- **Lag compensation.** The shooter's view tick is estimated from the acked snapshot (under one tick of error on average; game-glue.md).
- **Identity and lobby.** No accounts. A browser keeps a public id and a private token. Presence comes from `POST /api/presence` polls, not from the game connection. `/api/duel/answer` and the party `player` tag take no token, and the roster lists any id that polls.
- **Versions.** Protocol 6, `SIM_BEHAVIOR_VERSION` 6, rule set code format 5, generator version 4, `REPLAY_GOLDEN_HASH` `0x6d20_19b0_f52b_3f3f`, the nine mode replays and the 25 mood replays (`crates/ls-sim/tests/fixtures/mood_replays.txt`). The audit fix wave runs before M5 and may change the data hash (audit Q3); M5 starts from the state it leaves.

## 3. Design decisions for gaps in the design doc

Rows marked *Decided* follow a §8 answer. The other rows are the plan's design; a package that finds a better way updates its row here before the wave that needs it starts.

### 3.1 Desktop app (Tauri)

| Topic | Decision |
| --- | --- |
| Shape | *Decided* (questions 3 and 5). Tauri 2, Windows x64 only. The app shows the bundled web client (`web/dist`) in the system web view: WebView2, which is Chromium, so WebGPU and the whole client work as in Edge. The game core stays the WASM build inside the web view: there is one client code path, no new determinism surface, and no per-frame IPC of chunk meshes. The design doc's "native build of the core" becomes a later option, which the native = wasm32 golden checks would cover |
| Folder and build | `desktop/` with its own Cargo workspace (`desktop/src-tauri`, excluded from the root workspace), so `cargo test --workspace` does not compile Tauri and its web view crates. `desktop/package.json` holds `@tauri-apps/cli`. Builds go into `CARGO_TARGET_DIR=target/pkg`, like every package. A Windows-only `tools/ci.*` step builds it (`LS_SKIP_DESKTOP=1` skips it) |
| Network bridge | Rust commands `net_connect(url, kind)`, `net_send(bytes, reliable)` and `net_close()`, plus a Tauri `Channel` that pushes received packets to JS. On the web side, `web/src/net/tauri.ts` (`TauriTransport`) has the same surface as the other transports, and `connect.ts` picks it inside the app. The native clients come from `crates/ls-netio`, moved out of `ls-bots` (WebSocket and QUIC/WebTransport with trust by certificate hash). The app never needs the browser's secure-context rules, so it also reaches LAN servers on plain addresses over UDP |
| Native transport | *Decided* (question 4): QUIC datagrams through the existing WebTransport endpoint. That is UDP, encrypted, and needs no new server endpoint. At 30 snapshots a second (about 18 KB/s) QUIC's congestion control never limits the datagrams, so the latency equals raw UDP. The C2 package measures round trip and jitter for raw WebSocket, browser WebTransport and the app's QUIC on the fleet simulation. A raw UDP protocol (stateless-cookie handshake, ack bitfields, reliable fragments) is a later option only if the measurement shows a gap |
| HTTP from the app | Server list, directory and `/api` calls go through a Rust HTTP command when the web view would block them (mixed content, CORS). The app's origin is a local secure origin, so WebGPU works |
| Identity and settings | *Decided* (question 8). The same anonymous public id and private token as the browser, kept in the app's data folder (no accounts). Settings stay in the web view's storage |
| Window | Title, icon, a remembered size, F11 fullscreen, pointer lock (Chromium's, inside WebView2), no browser chrome. A strict CSP. The IPC allow-list holds only the bridge and HTTP commands. No remote content is loaded |
| Packaging | An NSIS per-user installer (no admin rights) with the WebView2 bootstrapper, and a portable zip. Target size under 30 MB |
| Distribution | *Decided* (question 6): M5 ships unsigned builds for testers; SmartScreen warns once, as the player manual will say. Code signing, the updater and its keys, and store publishing are set up by the maintainer separately, close to the finished product; M5 builds none of them |
| Deep links | First in the cut order. `loosescrews://join?...` opens an invite link in the app. The browser's invite page offers "Open in the app" when the app is installed |
| Testing | Headless rules apply. The bridge has an integration test against a real `ls-server` on `127.0.0.1` (no window). `TauriTransport` has unit tests with a fake IPC. A desktop smoke starts the built app with a window that is never shown (`--smoke`): the fake pointer lock is injected through Tauri's init script, the app connects to a local server, flies 20 s of scripted input, writes a JSON report and exits. If WebView2 does not render in a hidden window, the smoke checks the connection and the session only, and the maintainer's own install-and-play test covers the rest |

### 3.2 Browser support

| Topic | Decision |
| --- | --- |
| Transports | *Decided*: no WebRTC fallback. Browsers connect over WebTransport, with the existing WebSocket fallback; every supported browser has at least one of them. Connection order and `?transport=ws|wt` stay as they are |
| Renderer | *Decided* (question 7): WebGPU only, no WebGL2 renderer. Browsers and GPUs without WebGPU (no API, no adapter, no device) are not supported |
| Message without WebGPU | The existing error screen (`web/src/ui/errorScreen.ts`) states clearly that the game needs WebGPU, names the supported browsers, and, on a plain-`http://` page, explains that WebGPU needs a secure context (HTTPS or `localhost`) and points to the desktop app for LAN play. A1 builds it; the player manual's browser section says the same |

### 3.3 Weak GPUs

| Topic | Decision |
| --- | --- |
| Far terrain | B6 makes far terrain cheaper (fine detail and the second GGX lobe fade out with distance; far chunks shade with fewer samples). Targets on the integrated GPU at 1080p: every gallery scene within 33 ms at "low", sunlit caves within 40 ms at "medium". No regression on the RTX 5080 |

### 3.4 Regions, deployment and ops

| Topic | Decision |
| --- | --- |
| Orchestrator | *Decided* (question 2): **Nomad**. It is a single binary with a `raw_exec` / `exec` driver that runs `ls-server` without containers, allocates TCP and UDP ports, does health checks and rolling updates with a kill signal and timeout, and runs as a dev agent on Windows, so the job specs can be tested on this machine. Agones needs a Kubernetes cluster per region, which is far more than a handful of game servers needs. Nomad's licence (BUSL 1.1) allows running your own game servers. One Nomad region per cloud region, one datacenter each |
| Artifacts | `deploy/`: Nomad jobs for `ls-server` (per region: count, ports, `/healthz` check, `kill_signal` SIGTERM, `kill_timeout` long enough for a match plus intermission) and `ls-directory`; a `Caddyfile` (HTTPS for the web client, `/ws`, `/api`, and the directory on its own host or path; WebTransport UDP goes straight to the server); systemd units as the no-orchestrator fallback; a Dockerfile as an option. Linux release binaries come from a WSL Ubuntu the maintainer installs (no hosted CI, question 9) |
| Ops endpoints | `GET /healthz` (`200 ok`, `503 draining`); `GET /metrics` in the Prometheus text format (per room: tick p50/p99/max wall and CPU, players, bytes sent per client, full snapshots, matches, rejects, kicks, encode errors; per process: rooms, private rooms free, connections per transport); `--log-format json`. Metrics and health are read-only and carry no player names |
| Graceful drain | On SIGTERM (Ctrl+Break on Windows) or `--drain-after <s>` (tests), the server stops taking new public joins (`Reject "server draining"`) and leaves the directory. Running matches finish. Players get the results, then a `Disconnect` with the reason "server restarting", and the client offers Quick play. The process exits when its rooms are empty or `--drain-timeout` passes |
| Regions and Quick play | The directory already carries `--region`. It gains `GET /servers?region=` and a region list. The browser groups the server list by region with each region's best ping. A **Quick play** button on the start screen picks the region with the lowest measured ping (and anything within 30 ms of it), then the fullest public room with space, and moves on to the next region when everything is full. `ls-bots --quick-play` does the same |
| Local multi-region simulation | `tools/fleet.mjs` starts `ls-directory`, R regions x S servers (each `--region`) and bots on `127.0.0.1`. A test-only `ls-server --debug-latency-ms <n>` (it logs a warning like `--event-schedule`) delays every frame both ways on every transport, so a server "in another region" pings like one: the default fleet is eu +7 ms, us +45 ms, ap +90 ms one way. Quick play, the server list, drain and regional fill are tested against it |
| Real deployment | *Decided* (question 1): not in M5. The maintainer sets up hosting separately, later. M5 ends with the deploy artifacts, tested on a local Nomad dev agent, and the local multi-region simulation. Agents never store credentials in the repo or memory |
| Directory over HTTPS | `ls-server --directory-public <https url>`: the URL handed to browsers (presence and `/api/info` replies), separate from the heartbeat URL `--directory`. `ls-directory --trusted-proxy <ip>`: `X-Forwarded-For` is honoured only from that address, so rate limits apply per player, not per proxy |

### 3.5 Server hardening (M3 gaps)

| Gap | Decision |
| --- | --- |
| Duel answers without a token | `POST /api/duel/answer` takes the answering player's `token`, which must own the challenged id (403 otherwise) |
| Party tags without a token | Protocol 7 `Welcome` carries a 64-bit `session_key` (from the server's OS random source; never in the sim). `POST /api/party` with `player` must also send that connection's `session_key` |
| Fake presence registrations | The roster (server list, `/api/players`, challenges) lists only ids with an open game connection: `/api/presence` takes the `session_key` and binds the id to that connection. Presence without a connection still returns the inbox but is not listed. A per-IP cap on presence ids applies |
| Directory address squatting | The first claim of an address must prove it. The directory answers with a nonce, fetches `http://<address>/api/directory-proof?nonce=` and expects `sha256(secret ‖ nonce)`, and lists the entry only after that. A squatter cannot answer at someone else's address, and an address with no `ls-server` behind it is never listed |
| No backoff for failing next-room builds | A failing build is retried after 1, 2, 4 ... up to 60 s. After 3 failures the playlist skips to the next entry and warns. Players stay in the running room (results or warm-up) and are never dropped for a failed build |
| A panicked private room keeps its slot | The room thread runs under `catch_unwind`. A panic frees the slot, sends every link `Disconnect { "room failed" }`, logs at `error` and counts in `/metrics` |
| A join racing a closing private room | The `Hub` answers `Reject { "room closed" }` (`Reject`'s layout stays frozen) |
| Private rooms ignore `--ban` | `api::custom_rules` and `duel::duel_rules` resolve rule set codes through the server's `ServerRules` (balance list, then bans), like the public rotation, and the invite shows the effective code |

### 3.6 Bandwidth and lag compensation (protocol 7)

| Topic | Decision |
| --- | --- |
| New-projectile velocity | A projectile new against the baseline is predicted from its owner: velocity = owner's forward (from the replicated smallest-three orientation, rotated with `+`/`*` only, so native and WASM agree) x the weapon's muzzle speed from the data (the data hash covers it), with the fan index of spread weapons. Only the residual is sent. Fallback per projectile: today's residual against the owner's previous shot, or the raw velocity (2 bits pick). Target: the busiest snapshot within 576 B at 2, 4 and 6-tick baselines on the four seeds of `busy_fight_with_projectiles_fits_the_budget` |
| Exact view tick | `Input` carries the tick of what the client rendered for remote ships when it sampled the command, as a `u8` offset back from the command tick (clamped to the 250 ms rewind). Lag compensation uses it instead of the estimate, and the estimate stays only as a guard against forged values (the server clamps to what it could have sent) |
| 32 players | B5 runs the first live 32-player room (31 bots and a probe, Team Brawl) for 10 minutes over WebSocket and WebTransport: CPU tick p99 < 4 ms, per-client average within 17,280 B/s, the busiest snapshot recorded against 576 B, and the `Welcome` size after a heavy 10-minute match. Compression of `Welcome` only if it exceeds 256 KiB |

### 3.7 M4 leftovers

| Topic | Decision |
| --- | --- |
| Own-death kill-cam | *Decided* (question 11). After your own death, the client replays the last 3 s from the killer's view (the victim's view for kills by the level) in the existing kill-cam window, during the respawn wait. It is on by default in modes with respawns (Team Brawl, Paint the Void, Heist, Hot Reactor, runners in The Architect) and off in Belt Up (the hold is short and the course matters), The Squeeze and Last Color Standing. Any key skips it. It never delays the respawn, and the setting is in Options. Client only, no protocol change |
| Grumpy cave-ins | *Decided* (question 11). While the level is grumpy, the cave-in threshold is multiplied by `grumpy_threshold_scale` 0.7 (`data/world/instability.ron`), so cave-ins come about 1.4x as often. Telegraph, safety rules and the 8 s minimum interval stay. The mood is already in the state hash and in `MoodView`, so clients need nothing new |
| Bot stalls | A per-phase timing log in `ls-bots` (decode, session apply, think: navigation, aim, arsenal, objective; encode; terrain edits), then the fix. Bots send inputs for a new room from its `Welcome`, not from its first snapshot. That removes the transition-window starvation |
| Thin blade steps | The mesher handles 1.2-2 u blades seen along their thin side (the D5 note: above the thin-rock pass's 0.9 u). Budget +5 % mesh time on sums. Phone review of `cave-sunbeam` |
| Cave-in remesh benchmark | `crates/ls-game/tests/cave_in_remesh.rs` like `cat_paw_remesh.rs`: p99 per chunk within the client's 4 ms frame budget, measured in thread CPU time (§3.8) |

### 3.8 CI and timing

| Topic | Decision |
| --- | --- |
| Hosted CI | *Decided* (question 9): no hosted CI. `tools/ci.*` runs locally and is the gate for `main`; the pre-push hook runs it on every push to the public remote. Releases (Linux builds included) are built, signed and published locally; the GitHub release is created by hand. The master and the work folders stay as they are (ADR 0001) |
| CPU-time ticks | The sim thread measures its own CPU time per tick (Windows `QueryThreadCycleTime`, converted with the measured cycle rate, because `GetThreadTimes` only resolves about 15.6 ms; Linux `CLOCK_THREAD_CPUTIME_ID`) beside wall time. `tick stats` and `/metrics` carry both. The e2e p99 checks assert on CPU time. Wall-clock p99 becomes a warning outside quiet windows. The timing tests (`tectonic_tick`, `squeeze`, `cat_paw_remesh`, `cave_in_remesh`) measure thread CPU time too. That replaces the "dedicated quiet machine" of the carried item |
| Reference GPU | *Decided* (question 10): M5 rents no GPU. The maintainer measures on real hardware of the GTX 1660 / RX 6600 / M1 class separately (`gallery.mjs --label refgpu-<name>`). Until then the M5 timings stay "estimated", with the integrated-GPU floor re-measured by B6 |

## 4. Determinism and handshake impacts

| Item | Change | Owner of the change |
| --- | --- | --- |
| `GENERATOR_VERSION` | Unchanged (4). A2's thin-blade mesher change re-blesses the mesh goldens only (`surface_nets`, `edited`), without a version bump, as in M4 | A2 |
| `SIM_BEHAVIOR_VERSION` | 6 -> 7 for the grumpy cave-ins (version 6 is released on `main` with M4) | A2 assigns, integrator decides |
| `REPLAY_GOLDEN_HASH` and the nine mode replays | Unchanged: the scripted replays run at Absurdity 0 (no director, no mood) and Instability 0. Mood replays: lines whose level turns grumpy while cave-ins happen are re-blessed with that reason. A new `world/cave_in_grumpy` replay. Native, wasm32 and `determinism-check.mjs` must match | A2 |
| Data hash | `data/world/instability.ron` gains `grumpy_threshold_scale`, which changes the hash automatically. No other `data/` change is planned; muzzle speeds used by the new projectile predictor are already in the weapon files | A2 |
| Protocol | 6 -> 7 in wave A: the render-tick offset in `Input`, new-projectile velocity prediction (snapshot encoding), `Welcome::session_key`. `Reject`'s layout stays frozen, so old clients still show "please update". Additions before wave A integration stay in 7 | A3 (day-one wire layout note pushed first; A4 builds against it) |
| Transports | The app's QUIC bridge carries the same encoded `Message`s as WebTransport; no protocol change. The unreliable size limit stays 1200 B on every transport | A6 |
| Rule set codes, share codes | Unchanged (format 5, version 4 layout) | — |
| Desktop app | Runs the same WASM in the web view: no new determinism surface. A future native core would be covered by the existing native = wasm32 goldens | A6 |
| Directory | The claim proof (§3.5) and region filters change the directory API; servers and the directory ship from one build, and the directory stays independent of the game crates | A4, B4 |
| WASM cross-check | `tools/determinism-check.mjs` gains `world/cave_in_grumpy` | A2 |

## 5. Waves

M5 starts after the code-quality audit's eight fix packages are integrated (audit decision Q10). Each wave is a set of packages that run in parallel in separate work folders (`loosescrews.win2`+), one package per folder, as in M3 and M4. Packages in one wave touch disjoint files except under "Shared files"; the integrator (win1) merges them in the listed order. A package may start before its dependency is merged only against the listed day-one contract. "As built" notes are added under each wave during execution.

Package ids are kept stable: A5 (WebRTC fallback), A7 (WebGL2 spike and seam), B1 (WebGL2 renderer), C1 (WebGL2 effects) and C3 (real deployment) are dropped and have no replacement.

Rules for every package:
- Build with `CARGO_TARGET_DIR=target/pkg` (the desktop workspace too). The lead deletes `target/` in the package folders after each green wave.
- Browser runs are headless only, with `tools/browser-smoke/fake-pointer-lock.mjs` on every page, scratch scripts included. Automation never shows the desktop app on screen (§3.1 Testing).
- Packages that draw: gallery scenes, before and after runs (`gallery.mjs --label <pkg>-before|after`), within budget; the lead sends the shots for the phone review after each integrated wave.
- Packages that add a server flag update `--help` and leave the hosting guide to D3. Tests bind `127.0.0.1` (port 0 where possible).
- Credentials (cloud, DNS, GitHub, signing) are never written to the repo, the memory or a package prompt.

### Wave A — leftovers, protocol 7, hardening, desktop shell

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| A1 Web leftovers | Own-death kill-cam (§3.7: setting, modes, skip, gallery `killcam-own`); M3 D2 items (party polls stop after a disconnect, server table as cards below 600 px, weapon/hull panels at 360 px, lobby card focus); M4 D2 items (results-screen hum and ink particles stop, mood badge at phone width, iOS audio resume on the next touch, `ArchitectHud` updates in place); no `powerPreference` on Windows; the no-WebGPU message (§3.2) | `web/src/game/killcam.ts`, `web/src/ui/` (options, `party.ts`, `serverList.ts`, `hud.ts`, lobby card, `architectHud.ts`, `errorScreen.ts`), `web/src/audio/` (resume), `web/src/renderer/gpu.ts` (`powerPreferenceFrom` only), gallery | — |
| A2 Rust leftovers | Grumpy cave-ins (§3.7; `SIM_BEHAVIOR_VERSION` 7, data, replays, statistics test); the 1.2-2 u blade steps (mesher, mesh goldens); the cave-in remesh benchmark | `crates/ls-sim/src/instability.rs` (+ a mood read from `director/`), `data/world/instability.ron`, `crates/ls-sim/tests/fixtures/mood_replays.txt`, `crates/ls-worldgen/src/{mesh,chunk}.rs`, `crates/ls-game/tests/cave_in_remesh.rs` (new), `userDocs/reference/` (generated) | — |
| A3 Protocol 7 and bandwidth | Render-tick offset in `Input` and exact lag compensation; new-projectile velocity prediction; `Welcome::session_key`; tightened bandwidth tests | `crates/ls-net`, `crates/ls-game` (`client/`, `server/` incl. lag compensation, `convert.rs`), `crates/ls-wasm` (render tick into the session), one call in `web/src/game/` if the render tick comes from TS | — ; **day-one contract**: the protocol 7 layout note (fields and bit widths) pushed first |
| A4 Server and directory hardening | All §3.5 rows; `--directory-public`, `--trusted-proxy`; the session key in presence, party and duel requests | `crates/ls-server/src/{api,party,duel,rooms,sim,lobby,rules,directory}.rs`, `crates/ls-directory`, `web/src/net/social.ts` | A3 day-one contract (`session_key`) |
| A6 Desktop shell | §3.1: `desktop/` workspace, Tauri 2 window and CSP, `crates/ls-netio` (native clients moved from `ls-bots/src/transport.rs`; `ls-bots` uses it), the bridge commands and channel over QUIC datagrams, `web/src/net/tauri.ts`, the app branch in `connect.ts`, HTTP through Rust, identity in the app data folder, NSIS installer | `desktop/` (new), `crates/ls-netio` (new), `crates/ls-bots/src/transport.rs` (moved), `crates/ls-bots/Cargo.toml`, `web/src/net/{tauri,connect}.ts` (`tauri.ts` new), root `Cargo.toml` (member and `exclude`) | — |
| A8 Bot stalls | Per-phase timing in `ls-bots`, the Architect-match stall found and fixed, inputs from the `Welcome` | `crates/ls-bots` (except `transport.rs`) | — |
| A9 CI and timing | CPU time per tick in `ls-server` stats and `tick stats`; e2e checks on CPU p99; the timing tests in thread CPU time | `crates/ls-server/src/{stats,timing}.rs` (the measurement call in `sim.rs` is shared with A4), `tools/e2e.mjs`, `tools/ci.*`, `crates/ls-game/tests/` timing helper (shared with A2's new test) | — |

Shared files: `crates/ls-server/src/sim.rs` (A4: build backoff and panic handling; A9: one measurement call); `crates/ls-server/src/main.rs` flags (A4, A9); `crates/ls-bots` (A6 moves `transport.rs`, A8 owns the rest); `CLAUDE.md`, `docs/dev-guide.md` (integrator resolves).

Merge order: A3, A2, A4, A9, A6, A8, A1.

Acceptance (wave A):
- A1: online smoke in Team Brawl: after the own ship dies, the kill-cam plays at most 3 s, any key skips it, the respawn is never later than without it, it stays off in Belt Up, and the setting persists across a reload; gallery `killcam-own` within budget and phone review; the phone-width HUD scenes (360 x 740) show no overlaps (a layout test on element boxes); the party panel sends no poll after a disconnect (unit test); the smoke console has no `powerPreference` warning on Windows; with WebGPU disabled (headless smoke), the start shows the no-WebGPU message with the supported browsers, and on a plain-`http://` LAN address the secure-context explanation.
- A2: the grumpy statistics test over 200 simulated matches at Instability 60: cave-ins per grumpy minute 1.3-1.5x those per calm minute; none at Instability 0; the never-enclose property holds over 8 seeds; `REPLAY_GOLDEN_HASH` and the nine mode replays unchanged; `world/cave_in_grumpy` equal on native, wasm32 and `determinism-check.mjs`; `cave-sunbeam` blade drawn without steps (a unit test on a 1.5 u blade at 12 slants, gallery before and after), mesh time within +5 % on sums; `cave_in_remesh.rs` passes (p99 per chunk < 4 ms).
- A3: protocol 7 round trips for every changed item; on all four seeds the busiest snapshot is within 576 B at 2, 4 and 6-tick baselines; an in-process netplay test with jittered render ticks shows lag-compensated hits matching what the client drew, where the estimate misses some (recorded); `Reject` decodes for a protocol 6 client (test).
- A4: one regression test per §3.5 row that fails without its fix (forged duel answer 403; party tag with a wrong key 403; roster only from connected players; unproven or squatted directory claims not listed; build backoff 1-2-4 s then the next entry, with an injected build failure; a panicking private room frees its slot and disconnects with a reason; a join to a closing room gets `Reject`; custom and duel rooms apply `--ban` and the balance list); an `https` page gets the `--directory-public` URL; the directory rate-limits by `X-Forwarded-For` only from `--trusted-proxy`; `ls-server` social tests and `social.mjs` smoke still pass.
- A6: `cargo build --release` in `desktop/`; the bridge integration test against a real `ls-server` on `127.0.0.1` (QUIC: Hello, Welcome, 20 s of snapshots, 0 decode errors); `ls-bots` release tests pass on `ls-netio` (WebSocket and WebTransport); `TauriTransport` unit tests with a fake IPC; installer built and under 30 MB; the desktop smoke report (or a note why hidden rendering failed); the maintainer installs it and plays one match (phone screenshot).
- A8: `node tools/e2e.mjs --count 8 --playlist <Architect-heavy playlist> --duration 900` on a quiet machine: 0 input-starvation warnings and no bot step of 100 ms or more; the bots' first input in a new room arrives within 2 ticks of its `Welcome`; `ls-bots` release tests pass.
- A9: `tick stats` and the e2e show CPU p99 beside wall p99; under a synthetic all-core load the CPU-time check passes where the wall-clock check fails (recorded); the four timing tests pass under that load.

### Wave B — platforms and ops

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| B2 Desktop app complete | Deep links, log file, version display, the desktop smoke in `tools/`, the `tools/ci.*` desktop step | `desktop/`, `tools/desktop-smoke/` (new), `tools/ci.*` (one step), `web/src/ui/` invite page ("Open in the app") | A6 |
| B3 Server ops and deploy artifacts | §3.4 ops endpoints (`/healthz`, `/metrics`, drain, `--log-format json`); Linux release build; `deploy/` (Nomad jobs, Caddyfile, systemd units, Dockerfile); a local Nomad dev agent on Windows (`raw_exec`) deploys 2 servers and the directory, then a rolling update with drain | `crates/ls-server/src/{http,server,main}.rs` (endpoints, signals), `deploy/` (new), `tools/` (deploy check script) | A4, A9 |
| B4 Regions and fleet simulation | `tools/fleet.mjs`, `ls-server --debug-latency-ms`, directory region filter and list, Quick play and the server list grouped by region, `ls-bots --quick-play` | `tools/fleet.mjs` (new), `crates/ls-server/src/link.rs` (delay), `crates/ls-directory` (filters), `web/src/ui/{startScreen,serverList}.ts`, `crates/ls-bots/src/social.rs` | A4 |
| B5 32 players and bandwidth | Per-client bytes and the busiest snapshot in `tick stats` and `/metrics`; the 32-player e2e (§3.6) over WebSocket and WebTransport; `Welcome` size after a heavy match | `crates/ls-server/src/stats.rs` (counters), `crates/ls-game/src/server/` (byte counts), `tools/e2e.mjs` (32-player checks) | A3, A9 |
| B6 Weak-GPU pass | §3.3 far-terrain cost (detail fade, fewer far samples); integrated-GPU gallery at low and medium; phone review of near and far rock | `web/src/renderer/shaders/terrain.ts`, `web/src/renderer/quality.ts`, `web/src/renderer/renderer.ts` (per-chunk distance) | — |

Shared files: `crates/ls-server/src/stats.rs` (B3 reads, B5 adds counters); `crates/ls-server/src/main.rs` flags (B3, B4).

Merge order: B6, B3, B5, B4, B2.

Acceptance (wave B):
- B2: the desktop smoke passes from `tools/ci.ps1` (or is skipped with a reason); `loosescrews://join?...` opens the invite in the app (unit-tested parser plus the smoke); the app shows its version and writes a log file.
- B3: `/healthz` and `/metrics` tested (fields, no player names); drain test: during a match, SIGTERM -> no new public joins (`Reject "server draining"`), the match finishes, every player gets the results and the restart reason, the process exits; `nomad job validate` on every job; the local Nomad dev agent runs two servers and the directory, and a rolling update drains with 0 mid-match disconnects (bots); Linux binaries built (locally, e.g. in WSL) and their smoke run recorded.
- B4: fleet simulation eu/us/ap at +7/+45/+90 ms one way: the server list shows region pings within 10 ms of 14/90/180 ms; the smoke's Quick play joins eu; with eu full it joins us; 24 Quick-play bots fill eu first; a drained eu server's players reach another eu server through Quick play.
- B5: 32-player Team Brawl for 10 minutes over WebSocket and over WebTransport: CPU tick p99 < 4 ms in every window, per-client average within 17,280 B/s, the busiest snapshot recorded against 576 B, 0 desyncs, 0 protocol errors; the `Welcome` size after a 10-minute match with heavy cratering recorded (and compressed only above 256 KiB).
- B6: integrated GPU at 1080p: every gallery scene within 33 ms at "low", sunlit caves within 40 ms at "medium"; RTX 5080: no scene slower than before by more than 5 %; phone review of `cave-sunbeam` and `open-sky` before and after.

### Wave C — transport matrix

| Package | Work | Owner areas | Depends on |
| --- | --- | --- | --- |
| C2 Transport matrix | e2e with mixed transports in one room (`--transports ws,wt`); the round-trip and jitter comparison of question 4 on the fleet simulation (WebSocket, browser WebTransport, the app's QUIC); reconnect handling in the app's bridge | `crates/ls-netio`, `crates/ls-bots` (flag), `tools/e2e.mjs`, `tools/fleet.mjs` (measurement mode), `desktop/src-tauri` (reconnect) | A6, B4 |

Acceptance (wave C):
- C2: e2e with 8 bots split over ws and wt in one room, a full rotation: 0 desyncs, 0 protocol errors; the transport comparison table (round-trip p50/p99, jitter, under 1 % and 5 % loss) recorded, with the conclusion on raw UDP (a later option only if it shows a gap); the app reconnects after a server restart without restarting.

### Wave D — review, docs, soak, polish

| Package | Work |
| --- | --- |
| D1 Rust review | Review of A2-A4, A6, A8, A9, B3-B5, C2 with a security focus: the directory proof, session keys, drain, the app's IPC surface and CSP, protocol 7 decoding of hostile input; fixes with regression tests |
| D2 Web review | Review of A1, A6 (web parts), B2, B4, B6: transports and fallback order, the no-WebGPU message, Quick play, kill-cam, the app branch; fixes |
| D3 Docs | New `docs/architecture/platforms-and-ops.md` (transports: WebSocket, WebTransport, the app's bridge; the desktop app; browser support; deployment, Nomad, Caddy, drain, metrics, regions, the fleet simulation; CPU-time ticks); updates to netcode.md (protocol 7), server.md, directory.md, renderer.md (WebGPU only, the no-WebGPU message), determinism.md ("Milestone 5 record"), dev-guide.md (desktop build, fleet), `CLAUDE.md` (commands, layout: `desktop/`, `deploy/`, `crates/ls-netio`); `userDocs/player-manual.md` (the desktop app and SmartScreen, supported browsers and WebGPU, Quick play, transports in F3, the kill-cam setting), `userDocs/hosting-guide.md` (ports, Caddy and TLS, Nomad and systemd, drain, metrics, regions, `--directory-public`, `--trusted-proxy`, the directory proof, every new flag checked against `--help`); carried-over items closed in implementation-plan.md §8 |
| D4 Milestone soak | 60 minutes on a quiet machine: default rotation with The Architect and Belt Up at Absurdity 100, 8 bots split over ws and wt, a private duel and a private Architect room; next to it the fleet simulation (3 regions x 2 servers, 24 Quick-play bots) with one server drained mid-run; then a 10-minute 32-player Team Brawl. Checks as M4 plus CPU-time p99, per-client bandwidth and the drain |
| D5 Visual polish | The phone review list: the kill-cam, the weak-GPU far terrain, the no-WebGPU message, the desktop app (icon, window, installer pages) |

Acceptance (wave D): §6.

## 6. Acceptance criteria (milestone)

- The desktop app (Windows x64) installs per user without admin rights. It plays online over UDP through its native bridge (0 desyncs in the bridge test and the desktop smoke, or the maintainer's test where hidden rendering fails), plays offline practice, and lists and joins servers. The maintainer played a match in it. Builds are unsigned tester builds.
- Browser support: WebGPU only. A browser without WebGPU gets the clear message (smoke with WebGPU disabled); the transports are WebTransport and the WebSocket fallback.
- Weak GPUs: on the integrated GPU every gallery scene is within 33 ms at "low".
- Deployment: Nomad jobs, the Caddyfile and systemd units deploy `ls-server` and `ls-directory` on a local Nomad dev agent. `/healthz`, `/metrics` and the drain work. A rolling update drops no player mid-match. Linux release builds exist.
- Regions: the fleet simulation shows correct region pings, Quick play picks the best region and moves on when it is full, and bots spread the same way.
- Bandwidth: the busiest snapshot of the busy-fight test is within 576 B at 2, 4 and 6-tick baselines. A live 32-player room stays within 17,280 B/s per client on average with CPU tick p99 < 4 ms.
- Lag compensation uses the client's render tick (protocol 7) and stays within the 250 ms rewind.
- Hardening: every §3.5 gap has a regression test and is closed; private rooms and duels apply `--ban` and the balance list; `https` pages get an `https` directory URL.
- Carried-over items closed with evidence: bandwidth worst case, exact view tick, hosted CI (declined), CPU-time tick assertions, the `powerPreference` warning, own-death kill-cam, grumpy cave-ins. Reference mid-GPU timings are handed to the maintainer's measurement on real hardware (not an M5 gate); the integrated-GPU floor is re-measured.
- M4 exceptions closed: no bot stalls of 100 ms or more and 0 input-starvation warnings in the soak; the `cave-sunbeam` blade drawn without steps; a cave-in remesh benchmark within the frame budget.
- A 60-minute soak on a quiet machine (D4) with Absurdity 100, The Architect and Belt Up in the rotation, bots over WebSocket and WebTransport and the fleet simulation: every match reaches a result, 0 disconnects at match end (drained servers excepted, whose players get the reason), 0 desyncs, 0 protocol errors, at most 1 full snapshot per join or transition, CPU tick p99 < 4 ms per room, and the 32-player phase within budget.
- Versions changed deliberately and recorded in determinism.md: protocol 7, `SIM_BEHAVIOR_VERSION` 7, the re-blessed mood replays and mesh goldens; `REPLAY_GOLDEN_HASH` and the nine mode replays unchanged.
- docs/ and userDocs/ describe everything as built, including how to host a region and how to install the desktop app.

## 7. Risks

| Risk | Mitigation |
| --- | --- |
| Players whose browser or GPU lacks WebGPU cannot play | WebGPU ships in the browsers the game supports. The error screen names the requirement and the supported browsers, and the desktop app (WebView2) has WebGPU on Windows. A WebGL2 renderer stays out of scope |
| The desktop app cannot be tested headless like the browser | The bridge is tested without a window. The desktop smoke uses a never-shown window. The maintainer's install-and-play test is part of the criteria, and automation never opens a visible window |
| Unsigned builds trigger SmartScreen | Tester builds only; the player manual explains the warning. Signing comes with the maintainer's distribution setup, close to the finished product |
| No Docker, WSL or Linux machine for Linux builds | A WSL Ubuntu the maintainer installs (no hosted CI, question 9). Without it, the deployment artifacts stay Windows-tested and the Linux build moves to the maintainer's hosting setup |
| Credentials | M5 needs none: no cloud account, no rented machine, no signing keys (questions 1, 6, 10). Credentials stay with the maintainer and never reach the repo, the memory or a package prompt |
| Protocol 7 churns clients and bots together | One package (A3) owns the change with a day-one layout note; `Reject` stays frozen, so old clients show the update message |
| New network endpoints (metrics, health, directory proof) widen the attack surface | Rate limits, size caps, no player data in metrics, D1's security review, the M3 hardening in the same milestone |
| The shared machine distorts timing and soaks | CPU-time measurement (A9) for every assertion; soaks only when the machine is confirmed quiet |
| Scope: three platform tracks (desktop app, deployment and regions, bandwidth) plus the carried items | Cut order if time runs short (to be confirmed): deep links; the transport comparison under loss (C2) |

## 8. Decisions

All questions are answered; §1-§7 follow the decisions.

1. **Real cloud deployment in M5, or only deployable artifacts and a local multi-region simulation?** Decision: no real deployment in M5. The maintainer sets up hosting separately, later. M5 keeps the deploy artifacts (`deploy/`), tested on a local Nomad dev agent, and the local multi-region simulation.
2. **Orchestrator: Nomad or Agones?** Decision: Nomad, as proposed. It is one binary, runs `ls-server` without containers, does health checks and rolling updates with drain, and its dev agent runs on this Windows machine, so the jobs can be tested here. systemd units come along as the no-orchestrator fallback.
3. **Desktop app targets: Windows only, or also macOS and Linux?** Decision: Windows x64 only, as proposed. WebView2 is Chromium, so WebGPU and every tested browser feature work there. macOS and Linux are later options (Linux's web view, WebKitGTK, has no WebGPU).
4. **The app's "raw UDP": QUIC datagrams through the existing WebTransport endpoint, or a new raw UDP protocol?** Decision: QUIC datagrams, as proposed. They are UDP, encrypted, and reuse the server endpoint and the bots' tested client. C2 measures the latency; a raw UDP protocol is a later option only if the measurement shows a gap.
5. **The app's game core: the WASM build inside the web view, or a native build?** Decision: WASM in the web view, as proposed. One client code path, no new determinism surface and no per-frame transfer of chunk meshes; the app's Rust side does networking only. A native core is a later performance option.
6. **Code signing and distribution of the desktop app?** Decision: unsigned tester builds in M5, as proposed (SmartScreen warns once; the manual explains it). Code signing, the updater and its keys, and store publishing are set up by the maintainer separately, close to the finished product; M5 does not plan them.
7. **WebGL2: visual parity, or a reduced look?** Decision: no WebGL2 at all. The game is WebGPU only; browsers without WebGPU are not supported and get a clear message (§3.2).
8. **Accounts: still none?** Decision: none in M5, as proposed. The anonymous browser (and app) id and token stay, hardened with session keys (§3.5).
9. **Hosted CI: may the code go to a private GitHub repository with GitHub Actions?** Decision: no hosted CI. Verification is local (the pre-push hook runs the full check on pushes to the public remote), and releases are built, signed and published locally. Linux builds come from a WSL Ubuntu the maintainer installs.
10. **Reference mid-range GPU: hardware, or a rented cloud GPU?** Decision: no rented GPU. The maintainer measures on real hardware separately; until then the timings stay "estimated", with the integrated GPU as the floor.
11. **Confirm the two M4 items for wave A: the kill-cam after your own death, and the grumpy mood's extra cave-ins?** Decision: yes, both, as proposed. The kill-cam is on by default in respawn modes and off in Belt Up, The Squeeze and Last Color Standing, and any key skips it. Grumpy levels cave in about 1.4x as often.

Scope decisions beyond the questions:
- **WebRTC fallback for browsers without WebTransport.** Decision: dropped. Every supported browser has what the game needs; WebTransport with the existing WebSocket fallback stays.
- **WebGL2 renderer.** Decision: dropped (question 7).
