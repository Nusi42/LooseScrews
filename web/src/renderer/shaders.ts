// WGSL sources. Both pipelines share the per-frame uniform block below; keep
// FRAME_UNIFORM_FLOATS and the writer in renderer.ts in sync with it.

/** Number of paint team colour slots, indexed by team id; ids >= TEAM_SLOTS (NO_TEAM = 255) are unpainted. */
export const TEAM_SLOTS = 4;

/** Size of the Frame uniform struct in f32 units (192 bytes). */
export const FRAME_UNIFORM_FLOATS = 48;

const FRAME_STRUCT = /* wgsl */ `
struct Frame {
  view_proj: mat4x4f,
  camera_pos: vec4f,   // xyz camera position, w = time in seconds
  light_dir: vec4f,    // xyz unit vector towards the sun, w = headlight strength
  fog_color: vec4f,    // rgb fog colour, w = exp2 fog density
  rock_color: vec4f,   // rgb unpainted surface colour
  team_colors: array<vec4f, ${TEAM_SLOTS}>,
};
@group(0) @binding(0) var<uniform> frame: Frame;

fn apply_fog(color: vec3f, world_pos: vec3f) -> vec3f {
  let d = distance(world_pos, frame.camera_pos.xyz);
  let fd = frame.fog_color.w * d;
  let fog = 1.0 - exp2(-fd * fd);
  return mix(color, frame.fog_color.rgb, clamp(fog, 0.0, 1.0));
}

fn light(n: vec3f, world_pos: vec3f) -> vec3f {
  let sun = max(dot(n, frame.light_dir.xyz), 0.0) * vec3f(1.0, 0.95, 0.85);
  // Hemisphere ambient: slightly brighter from above.
  let ambient = mix(vec3f(0.10, 0.10, 0.13), vec3f(0.22, 0.24, 0.30), n.y * 0.5 + 0.5);
  // Headlight so tunnels stay readable: falls off with distance from the camera.
  let to_cam = frame.camera_pos.xyz - world_pos;
  let d = length(to_cam);
  let head = frame.light_dir.w * max(dot(n, to_cam / max(d, 1e-4)), 0.0) / (1.0 + 0.004 * d * d);
  return ambient + sun * 0.8 + vec3f(head);
}
`;

export const TERRAIN_WGSL = /* wgsl */ `
${FRAME_STRUCT}

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world_pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) @interpolate(flat) team: u32,
};

@vertex
fn vs_main(@location(0) pos: vec3f, @location(1) normal: vec3f, @location(2) team: u32) -> VsOut {
  var out: VsOut;
  out.clip = frame.view_proj * vec4f(pos, 1.0);
  out.world_pos = pos;
  out.normal = normal;
  out.team = team;
  return out;
}

// Cheap value-noise-ish variation so flat surfaces show motion parallax.
fn grain(p: vec3f) -> f32 {
  let q = floor(p * 0.5);
  return fract(sin(dot(q, vec3f(12.9898, 78.233, 37.719))) * 43758.5453);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let n = normalize(in.normal);
  var base = frame.rock_color.rgb * (0.85 + 0.3 * grain(in.world_pos));
  var emissive = vec3f(0.0);
  if (in.team < ${TEAM_SLOTS}u) {
    let tint = frame.team_colors[in.team].rgb;
    base = mix(base, tint, 0.85);
    emissive = tint * 0.12; // paint glows faintly so it reads in dark tunnels
  }
  let color = base * light(n, in.world_pos) + emissive;
  return vec4f(apply_fog(color, in.world_pos), 1.0);
}
`;

export const INSTANCED_WGSL = /* wgsl */ `
${FRAME_STRUCT}

struct VsOut {
  @builtin(position) clip: vec4f,
  @location(0) world_pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) color: vec4f,
};

fn quat_rotate(q: vec4f, v: vec3f) -> vec3f {
  let t = 2.0 * cross(q.xyz, v);
  return v + q.w * t + cross(q.xyz, t);
}

@vertex
fn vs_main(
  @location(0) pos: vec3f,
  @location(1) normal: vec3f,
  @location(2) offset_scale: vec4f, // xyz world position, w = uniform scale
  @location(3) rotation: vec4f,     // unit quaternion (x, y, z, w)
  @location(4) color: vec4f,        // rgb colour, a = emissive amount 0..1
) -> VsOut {
  var out: VsOut;
  let world = quat_rotate(rotation, pos * offset_scale.w) + offset_scale.xyz;
  out.clip = frame.view_proj * vec4f(world, 1.0);
  out.world_pos = world;
  out.normal = quat_rotate(rotation, normal);
  out.color = color;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
  let n = normalize(in.normal);
  let lit = in.color.rgb * light(n, in.world_pos);
  let color = mix(lit, in.color.rgb * 1.5, in.color.a);
  return vec4f(apply_fog(color, in.world_pos), 1.0);
}
`;
