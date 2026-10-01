# AGENTS.md

Guidance for coding agents working on **Camel Eon**, a 2.5D jungle platformer
in Rust. The hero, Konrad (Flashback-inspired, purple velour tracksuit, big
blue shader-rendered hair, a Mega Man-style arm cannon firing blue plasma),
runs at speed over the rolling hills and loops of an alien wilderness (the
first level), jumps over the rooftops of a future city at dusk (the second)
and through the jungle (the third).
Graphics are GPU vector graphics (Vello on wgpu), not sprites. It runs in a
window on desktop, on the web (WebGPU), on Android (native, Vulkan) and in
the kitty terminal.

Live web build: https://nergnezor.github.io/CamelEon/ (deployed by
`.github/workflows/pages.yml` on every push to `main`, together with the Android
APK at https://nergnezor.github.io/CamelEon/camel-eon.apk).

## Conventions

- **All code, comments, identifiers and commit messages are in English.** The
  maintainer may chat in Swedish; the code stays English regardless.
- Match the surrounding style: doc comments on items, short comments that
  explain *why*, no commented-out code.
- The frame budget is **60 Hz on mobile**. Measure (see below) before and after
  anything that adds drawing work.
- Commit and push only when asked. Pushing to `main` deploys the web build.

## Environment and commands

The maintainer is on NixOS. `shell.nix` provides the runtime libraries (Wayland,
X11, Vulkan, udev); `.envrc` loads it with direnv. Without direnv, wrap commands
in `nix-shell --run '…'`.

| Task | Command |
|---|---|
| Play (window) | `cargo run --release` |
| Hot reload | `dx serve --hotpatch` (press `r` in its terminal to rebuild and restart) |
| Terminal mode | `cargo run --release -- --terminal` (kitty) or `--direct` (over SSH) |
| Web build | `web/build.sh` → `dist/` (needs `wasm-bindgen-cli` matching `Cargo.lock`) |
| Check the wasm target | `cargo build --release --target wasm32-unknown-unknown` |
| Android APK | `android/build.sh` → `dist/camel-eon.apk` (needs the SDK, NDK, `cargo-apk` and the `aarch64-linux-android` target) |
| Screenshot / test | `cargo run --release -- --snapshot out.ppm 'SCRIPT'` |

Before committing, make sure both the native build and the wasm target compile
without warnings (and the Android build, when touching `window.rs` or adding
modules).

### Hot reload limits

`dx serve --hotpatch` (subsecond) patches function bodies into the running
game. It **cannot** change struct layouts or things created at startup (the GPU
pipelines, the hair renderer). After such changes the running instance keeps the
old behaviour: restart it with `r`. When a user reports "nothing changed", this
is the first thing to rule out.

### Snapshot tool (the main way to verify visual changes)

`--snapshot OUT.ppm 'SCRIPT'` runs the game headless with scripted input and
saves the last frame (convert with e.g. `magick out.ppm out.png`).

- Script steps: `KEYS:seconds`, where KEYS is any of `L R J F` (left, right,
  jump, fire) or `-`/empty for none, e.g. `-:1 R:1.2 RJ:0.3`.
  Keys are held for the step and released at the next one (jump fires on
  release, or by itself once fully charged after 0.2 s).
- `@N` warps to checkpoint N first, e.g. `@4 R:0.7` (open ground, high speed).
- Environment: `CAMEL_EON_SIZE=WxH`, `CAMEL_EON_DETAIL=0..2`,
  `CAMEL_EON_HAIR=shader|vector|both`, `CAMEL_EON_WEATHER=rain|clear`,
  `CAMEL_EON_GPU_BENCH=1` (times the GPU frame and variants with parts skipped),
  `CAMEL_EON_WAV=out.wav` (renders the script's sound and prints its peak/RMS),
  `CAMEL_EON_LEVEL=N` (0 = the wilds, the default; 1 = the dusk city;
  2 = the jungle).
- It prints the scene build time, the paths/segments/clip layers per layer
  (what Vello has to work through), and a status line per step (position,
  velocity, state), which is handy for checking movement numerically.
- It renders a frame of a fresh game before the real one, so stale GPU caches
  show up in snapshots too.

Only kill processes you started yourself; the maintainer usually has the game
running.

## Architecture

`src/main.rs` picks the mode; everything else is shared. `src/android.rs` is
the crate root of the Android library (`android/Cargo.toml`, a separate
package so the desktop crate and `dx serve` don't see it): new modules must be
declared there too.

| File | Role |
|---|---|
| `game.rs` | Game state and update loop: player, camera (look-ahead and zoom with speed), hair spring, breathing, effects; `draw` fills the layers |
| `player.rs` | Movement: running with momentum boost, charged jump, coyote time, catching flies; running along hills and round loops |
| `level.rs` | Levels (`level::all()`, in order) and their theme: blocks, hills, loops, trees or props, jellies, banners, flies, checkpoints |
| `konrad.rs` | The hero: skeleton, animation clips (idle, run, air, crouch), drawing in a fixed layer order, hair strands and beard |
| `rig.rs` | Bones, poses (slerp blending) and forward kinematics |
| `canvas3d.rs` | Perspective camera and painter's-sorted 3D drawing onto a Vello scene |
| `jungle.rs` | The jungle: parallax background, platforms with grass, trees, near foreground, energy cells, global wind |
| `wilds.rs` | The wilds (Scavengers Reign-like): pastel sky with a ringed planet, spires, sky jellies, wrecks; hills in strata with moss, alien flora, pearl-shell loops, spores, bud checkpoints, the gate |
| `dusk.rs` | The dusk city: mesas, skylines, airship, maglev, traffic, industry; buildings, containers, catwalks, props; long shadows projected along the sunlight |
| `noise.rs` | Deterministic 1D/2D gradient noise, fbm and ridged noise for irregular shapes |
| `soft.rs` | Soft bodies: jellies (shape matching + pressure; Konrad bounces off them) and cloth banners (verlet grid, shaded per cell by its normal) |
| `weather.rs` | Rain, wind gusts, fog, pit mist, leaves, fireflies, birds |
| `paint.rs` | Drawing helpers and detail levels |
| `frame.rs` | `FrameRenderer`: renders the layers and runs the post passes |
| `audio.rs` | Procedural sound: `Sfx` events and `Ambience` from the game, a small synth, cpal output |
| `grass.rs` | Shader grass: blades packed as patches into a 2048×1024 atlas by a wgpu pass, each patch drawn by Vello as an image at its depth |
| `hair.rs` | Shader hair: strands → wgpu pass → texture drawn by Vello as an image |
| `window.rs` | winit window (desktop and web), input, adaptive resolution |
| `terminal.rs` | kitty graphics protocol frontend |
| `snapshot.rs` | Headless snapshot and GPU benchmark tool |
| `touch.rs`, `gamepad.rs`, `stats.rs` | Touch controls, gamepads (gilrs), FPS overlay |

### Frame pipeline

1. `Game::draw` fills three Vello scenes (`frame::Layers`): `far` and `mid`
   (background, rendered at half resolution) and `front` (world, Konrad, HUD,
   at full resolution). It returns `FrameInfo` with the hair strands and post
   settings.
2. The hair pass draws the strands into a 512² texture, and the grass pass
   draws the blades (instanced) into the grass atlas.
3. Vello renders each layer to its own texture.
4. A quarter-resolution pass does light shafts and bloom; a composite pass
   does depth of field (blurring far/mid), the dusk grade, vignette, sun glow
   and the rain grade. In the dusk city (`Post::theme`) a sky pass runs first,
   at half resolution: a sunset gradient, the sun and fbm clouds, only where
   the far layer (rendered transparent) leaves gaps. The light and composite
   passes read it from a texture; per-pixel noise at full resolution cost a
   mobile GPU half its frame rate.

### Gotchas

- Vello's `render_to_texture` needs `Rgba8Unorm` targets with
  `STORAGE_BINDING`, and writes straight (not premultiplied) alpha.
- A texture registered with Vello (`register_texture`) is cached in its image
  atlas: call `mark_override_image_dirty` whenever its contents change.
- A texture can't be sampled and rendered to in the same pass; the light and
  composite passes have separate bind groups for this reason.
- Konrad is drawn as one group in a **fixed part order**, not depth-sorted per
  part: per-part sorting made parts flicker in front of each other.
- kurbo panics in debug builds on `line_to` without a preceding `move_to`.
- Sound runs in cpal's callback, outside `hot()`: hot reload doesn't patch
  the synth. The game only queues `Game::sounds` and reports `ambience()`;
  the frontend passes them on with `Audio::feed`. On the web the audio
  device opens on the first key press or touch (browser autoplay rules).
- The game follows the display's refresh rate (`Game::update` takes any `dt`),
  so 90/120 Hz screens get 90/120 FPS. On Android the app asks for 120 Hz with
  `ANativeWindow_setFrameRate`; the adaptive resolution still only aims for 60.
- Android drops the window when the app goes to the background: `suspended`
  drops the render state and `resumed` rebuilds it.
- On Android, panics and `show_error` go to logcat: `adb logcat -s camel-eon`.
  The library is linked for 16 KB pages (`android/build.sh` sets RUSTFLAGS;
  cargo-apk ignores rustflags in `.cargo/config.toml`), or it won't load on
  newer devices.
- The dusk city honours the detail levels (see `dusk.rs`), so the adaptive
  quality can lighten it when the CPU is the limit.
- Dusk city shadows are geometry: `dusk::shadow_path` slides a caster's
  points along the sunlight onto a roof's plane and fills their hull, clipped
  to the roof and drawn just above it (`block_depth - 0.004`). Konrad's is cast
  from his solved skeleton. Keep the shadow sun (`dusk::sun_dir`) roughly
  consistent with the sun on screen (`dusk::SUN`).
- Soft bodies step at the game's fixed 1/120 s. Banners are verlet: moving a
  point also moves its velocity, so anything that pushes cloth must shift
  `prev` along too, or the cloth gets flung. Jellies stand at z = 0.15, just
  behind Konrad, so he passes in front of them (they only jiggle).
- Keys: N skips to the next level (window and terminal). Fire is X, J or
  Ctrl (terminal: x or j; gamepad: X/Square or R2; touch: the small button
  left of jump). A press shoots at once; holding charges a big shot, fired
  on release.
- Hills (`level::Hill`) are smooth curves, not boxes: on one Konrad runs
  along the slope, and only leaves it over a crest sharper than
  `GROUND_GRIP` allows at his speed. Loops (`level::Loop`) are entered by
  crossing their bottom on the ground; he falls off when
  speed² < g·r·sin(angle). He's drawn tilted (`Player::tilt`), so anything
  placed relative to his body should use `Player::up`.
- Web: Vello needs compute shaders, so the web build needs WebGPU; wasm is
  single-threaded, and there's no hot reload.
