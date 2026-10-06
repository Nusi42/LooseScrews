// Wrong genre's side view (M4 C1): the camera looks at the ship's plane
// from the side with its near plane just in front of it, so rock between
// the camera and the plane is cut away. Where the cut opens solid rock the
// terrain's back faces show (the "cutaway" terrain pipeline draws both
// faces); the terrain shader paints them with `cut_fill`: a flat stone
// brick fill in the plane's x/y (the plane is always perpendicular to z),
// like the ground of a side-scroller, darker toward its depth.

/** Brick size (u) of the cut fill: width and height. */
export const CUT_BRICK_U = [1.6, 0.8] as const;

/** WGSL: `cut_fill(p)` (linear HDR colour of the cut rock at world point `p`). */
export const CUT_WGSL = /* wgsl */ `
fn cut_hash(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn cut_fill(p: vec3f) -> vec3f {
  let b = vec2f(${CUT_BRICK_U[0]}, ${CUT_BRICK_U[1]});
  let row = floor(p.y / b.y);
  let x = p.x / b.x + 0.5 * (row - 2.0 * floor(row * 0.5));
  let cell = vec2f(floor(x), row);
  let g = vec2f(fract(x), fract(p.y / b.y));
  let mortar = min(min(g.x, 1.0 - g.x) * b.x, min(g.y, 1.0 - g.y) * b.y);
  let brick = smoothstep(0.03, 0.08, mortar);
  let tone = 0.75 + 0.5 * cut_hash(cell);
  // A lit top edge on every brick (light from above in the side view).
  let top = smoothstep(0.75, 0.95, g.y) * brick;
  // Fixed brick colours (linear HDR): warm red-brown bricks, dark mortar, a lit top edge.
  let brick_col = vec3f(0.30, 0.075, 0.032) * tone;
  let mortar_col = vec3f(0.018, 0.014, 0.012);
  let lit = vec3f(0.62, 0.3, 0.14);
  return mix(mortar_col, mix(brick_col, lit, top * 0.8), brick);
}
`;
