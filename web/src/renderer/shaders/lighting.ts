// Scene lighting shared by terrain and objects: sun, hemisphere ambient and
// the camera headlight, in linear HDR units. VQ6 (dynamic lights) extends
// this file; terrain and objects call `light(n, world_pos)` only.

export const LIGHTING_WGSL = /* wgsl */ `
fn light(n: vec3f, world_pos: vec3f) -> vec3f {
  let sun = max(dot(n, frame.light_dir.xyz), 0.0) * vec3f(1.0, 0.93, 0.8);
  // Hemisphere ambient: cool sky light from above, dim bounce from below.
  let ambient = mix(vec3f(0.035, 0.035, 0.05), vec3f(0.11, 0.12, 0.16), n.y * 0.5 + 0.5);
  // Headlight so tunnels stay readable: falls off with distance from the camera.
  let to_cam = frame.camera_pos.xyz - world_pos;
  let d = length(to_cam);
  let head = frame.light_dir.w * max(dot(n, to_cam / max(d, 1e-4)), 0.0) / (1.0 + 0.004 * d * d);
  return ambient + sun * 0.9 + vec3f(1.0, 0.97, 0.92) * head;
}
`;
