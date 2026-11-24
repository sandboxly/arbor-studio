//! Depth-buffered drawing of the shaded tree in the viewport (OpenGL through egui's glow
//! backend). The CPU still projects every vertex (so wind, selection colours and picking stay
//! exactly as before); the GPU rasterises with a real depth test, so crossing and touching
//! surfaces (branch joints, root crowns) are always drawn in the right order, and lights each
//! pixel, so normal maps show their surface detail.

use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Floats per vertex: clip position (4), unlit colour (3), highlight overlay rgb + amount (4),
/// uv (2), normal (3), world position (3), kind (1: 0 bark, 1 leaf, 2 already lit), position on
/// the material gradient's axis (1, see [`crate::grow::Mesh::axis`]), the parent's texture
/// coordinates and their weight where a branch's bark cross-fades into its parent's (3, see
/// [`crate::grow::Mesh::xuv`]).
pub const STRIDE: usize = 24;

/// Triangles drawn with the same textures.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupKey {
    pub color: Option<egui::TextureId>,
    pub normal: Option<egui::TextureId>,
    pub flip_green: bool,
    /// Normal strength × 1000 (kept as an integer so the key can be hashed).
    pub strength_milli: u32,
    /// Material gradients: the next layer's textures, blended in by `layer - band`.
    pub blend: Option<BlendKey>,
}

/// The second material of a blended group and its transition (f32 bit patterns, for hashing):
/// where it is, half the ramp's width, how far the noise moves it and the noise's size.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlendKey {
    pub at: u32,
    pub half: u32,
    pub reach: u32,
    pub scale: u32,
    pub color: Option<egui::TextureId>,
    pub normal: Option<egui::TextureId>,
    pub flip_green: bool,
    pub strength_milli: u32,
}

/// One frame's worth of geometry for the GPU.
#[derive(Default)]
pub struct GpuMesh {
    pub verts: Vec<f32>,
    /// Triangle index lists per texture set.
    pub groups: HashMap<GroupKey, Vec<u32>>,
    /// Direction towards the light (normalised).
    pub light: [f32; 3],
    pub sun: [f32; 3],
    pub ambient: [f32; 3],
    pub sky: [f32; 3],
    pub unlit: bool,
}

struct Res {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    u_tex: Option<glow::UniformLocation>,
    u_nrm: Option<glow::UniformLocation>,
    u_use_tex: Option<glow::UniformLocation>,
    u_use_nrm: Option<glow::UniformLocation>,
    u_green: Option<glow::UniformLocation>,
    u_strength: Option<glow::UniformLocation>,
    u_light: Option<glow::UniformLocation>,
    u_sun: Option<glow::UniformLocation>,
    u_amb: Option<glow::UniformLocation>,
    u_sky: Option<glow::UniformLocation>,
    u_unlit: Option<glow::UniformLocation>,
    u_tex2: Option<glow::UniformLocation>,
    u_nrm2: Option<glow::UniformLocation>,
    u_use_tex2: Option<glow::UniformLocation>,
    u_use_nrm2: Option<glow::UniformLocation>,
    u_green2: Option<glow::UniformLocation>,
    u_strength2: Option<glow::UniformLocation>,
    u_blend: Option<glow::UniformLocation>,
    u_ramp: Option<glow::UniformLocation>,
}

/// Lazily created GL resources, shared with the paint callbacks.
#[derive(Clone, Default)]
pub struct GpuRenderer {
    res: Arc<Mutex<Option<Result<Res, String>>>>,
}

impl GpuRenderer {
    /// A paint callback drawing `mesh` into `rect` with a depth buffer.
    pub fn callback(&self, rect: egui::Rect, mesh: Arc<GpuMesh>) -> egui::PaintCallback {
        let res = self.res.clone();
        egui::PaintCallback {
            rect,
            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                let gl = painter.gl();
                let mut guard = res.lock().unwrap();
                if guard.is_none() {
                    let r = unsafe { Res::new(gl) };
                    if let Err(e) = &r {
                        eprintln!("viewport shader: {e}");
                    }
                    *guard = Some(r);
                }
                let Some(Ok(r)) = guard.as_ref() else {
                    return;
                };
                unsafe { r.draw(gl, painter, &info, &mesh) };
            })),
        }
    }
}

const ATTRS: [(&str, i32); 9] = [("a_pos", 4), ("a_base", 3), ("a_hl", 4), ("a_uv", 2), ("a_nrm", 3), ("a_wpos", 3), ("a_kind", 1), ("a_layer", 1), ("a_xuv", 3)];

impl Res {
    unsafe fn new(gl: &glow::Context) -> Result<Res, String> {
        unsafe {
            let ver = egui_glow::ShaderVersion::get(gl);
            let new = ver.is_new_shader_interface();
            let decl = ver.version_declaration();
            let (vin, vout, fin) = if new { ("in", "out", "in") } else { ("attribute", "varying", "varying") };
            let (frag_out_decl, frag_out, tex_fn) = if new { ("out vec4 out_color;", "out_color", "texture") } else { ("", "gl_FragColor", "texture2D") };
            let precision = if ver.is_embedded() { "#extension GL_OES_standard_derivatives : enable\nprecision highp float;" } else { "" };
            let vs = format!(
                "{decl}\n{precision}\n\
                {vin} vec4 a_pos;\n{vin} vec3 a_base;\n{vin} vec4 a_hl;\n{vin} vec2 a_uv;\n{vin} vec3 a_nrm;\n{vin} vec3 a_wpos;\n{vin} float a_kind;\n{vin} float a_layer;\n{vin} vec3 a_xuv;\n\
                {vout} vec3 v_base;\n{vout} vec4 v_hl;\n{vout} vec2 v_uv;\n{vout} vec3 v_nrm;\n{vout} vec3 v_wpos;\n{vout} float v_kind;\n{vout} float v_layer;\n{vout} vec3 v_xuv;\n\
                void main() {{ gl_Position = a_pos; v_base = a_base; v_hl = a_hl; v_uv = a_uv; v_nrm = a_nrm; v_wpos = a_wpos; v_kind = a_kind; v_layer = a_layer; v_xuv = a_xuv; }}\n"
            );
            let fs = format!(
                r#"{decl}
{precision}
uniform sampler2D u_tex;
uniform sampler2D u_nrm;
uniform float u_use_tex;
uniform float u_use_nrm;
uniform float u_green;
uniform float u_strength;
uniform vec3 u_light;
uniform vec3 u_sun;
uniform vec3 u_amb;
uniform vec3 u_sky;
uniform float u_unlit;
uniform sampler2D u_tex2;
uniform sampler2D u_nrm2;
uniform float u_use_tex2;
uniform float u_use_nrm2;
uniform float u_green2;
uniform float u_strength2;
uniform float u_blend;
uniform vec4 u_ramp;
{fin} float v_layer;
{fin} vec3 v_xuv;
{fin} vec3 v_base;
{fin} vec4 v_hl;
{fin} vec2 v_uv;
{fin} vec3 v_nrm;
{fin} vec3 v_wpos;
{fin} float v_kind;
{frag_out_decl}
float hash13(vec3 p) {{
  vec3 q = fract(p * 0.1031);
  q += dot(q, q.yzx + 31.32);
  return fract((q.x + q.y) * q.z);
}}
float vnoise(vec3 p) {{
  vec3 i = floor(p);
  vec3 f = p - i;
  vec3 u = f * f * (3.0 - 2.0 * f);
  float x00 = mix(hash13(i), hash13(i + vec3(1.0, 0.0, 0.0)), u.x);
  float x10 = mix(hash13(i + vec3(0.0, 1.0, 0.0)), hash13(i + vec3(1.0, 1.0, 0.0)), u.x);
  float x01 = mix(hash13(i + vec3(0.0, 0.0, 1.0)), hash13(i + vec3(1.0, 0.0, 1.0)), u.x);
  float x11 = mix(hash13(i + vec3(0.0, 1.0, 1.0)), hash13(i + vec3(1.0, 1.0, 1.0)), u.x);
  return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}}
// a colour texture, cross-faded with the parent's mapping where a branch joins it
vec4 xsamp(sampler2D s) {{
  vec4 a = {tex_fn}(s, v_uv);
  if (v_xuv.z > 0.001) a = mix(a, {tex_fn}(s, v_xuv.xy), v_xuv.z);
  return a;
}}
// a normal map's tangent-space normal at `uv`, green and strength applied
vec3 nsamp(sampler2D s, vec2 uv, float green, float strength) {{
  vec3 tn = {tex_fn}(s, uv).xyz * 2.0 - 1.0;
  tn.y *= green;
  tn.xy *= strength;
  return tn;
}}
// bend normal `n` by tangent-space `tn` in the frame of texture coordinates `uv`
// (from screen-space derivatives of position and uv: no tangents needed)
vec3 bend(vec3 n, vec2 uv, vec3 tn) {{
  vec3 dp1 = dFdx(v_wpos);
  vec3 dp2 = dFdy(v_wpos);
  vec2 duv1 = dFdx(uv);
  vec2 duv2 = dFdy(uv);
  vec3 dp2perp = cross(dp2, n);
  vec3 dp1perp = cross(n, dp1);
  vec3 T = dp2perp * duv1.x + dp1perp * duv2.x;
  vec3 B = dp2perp * duv1.y + dp1perp * duv2.y;
  if (dot(T, T) > 1e-36 && dot(B, B) > 1e-36) {{
    return normalize(normalize(T) * tn.x + normalize(B) * tn.y + n * max(tn.z, 0.05));
  }}
  return n;
}}
void main() {{
  // material gradient: weight of the next layer's material at this pixel
  // (u_ramp = where the transition is, half its ramp, how far noise moves it, noise size)
  float bw = 0.0;
  if (u_blend > 0.5) {{
    float nz = 0.0;
    if (u_ramp.z > 0.0) {{
      vec3 q = v_wpos / max(u_ramp.w, 0.01);
      nz = clamp(((vnoise(q) * 0.65 + vnoise(q * 2.7 + 31.0) * 0.35) * 2.0 - 1.0) * 1.6, -1.0, 1.0);
    }}
    float x = v_layer + u_ramp.z * nz;
    if (u_ramp.y <= 1e-6) {{
      bw = x >= u_ramp.x ? 1.0 : 0.0;
    }} else {{
      float t = clamp((x - (u_ramp.x - u_ramp.y)) / (2.0 * u_ramp.y), 0.0, 1.0);
      bw = t * t * (3.0 - 2.0 * t);
    }}
  }}
  vec4 t = vec4(1.0);
  if (u_use_tex > 0.5) {{
    t = xsamp(u_tex);
    if (t.a < 0.5) discard;
  }}
  if (bw > 0.0) {{
    vec4 t2 = u_use_tex2 > 0.5 ? xsamp(u_tex2) : vec4(1.0);
    t = mix(t, t2, bw);
  }}
  vec3 n = normalize(v_nrm);
  if ((u_use_nrm > 0.5 || (bw > 0.0 && u_use_nrm2 > 0.5)) && v_kind < 1.5) {{
    // B points down the image (texture rows grow downwards): OpenGL maps store green = up
    vec3 tn = u_use_nrm > 0.5 ? nsamp(u_nrm, v_uv, u_green, u_strength) : vec3(0.0, 0.0, 1.0);
    if (bw > 0.0) {{
      vec3 tn2 = u_use_nrm2 > 0.5 ? nsamp(u_nrm2, v_uv, u_green2, u_strength2) : vec3(0.0, 0.0, 1.0);
      tn = mix(tn, tn2, bw);
    }}
    vec3 n1 = bend(n, v_uv, tn);
    if (v_xuv.z > 0.001) {{
      // the same in the parent's mapping, with its own tangent frame
      vec3 tx = u_use_nrm > 0.5 ? nsamp(u_nrm, v_xuv.xy, u_green, u_strength) : vec3(0.0, 0.0, 1.0);
      if (bw > 0.0) {{
        vec3 tx2 = u_use_nrm2 > 0.5 ? nsamp(u_nrm2, v_xuv.xy, u_green2, u_strength2) : vec3(0.0, 0.0, 1.0);
        tx = mix(tx, tx2, bw);
      }}
      n1 = normalize(mix(n1, bend(n, v_xuv.xy, tx), v_xuv.z));
    }}
    n = n1;
  }}
  vec3 l = normalize(u_light);
  vec3 k;
  if (v_kind > 1.5 || u_unlit > 0.5) {{
    k = vec3(1.0);
  }} else if (v_kind > 0.5) {{
    k = u_amb * 1.17 + u_sky * 0.7 * abs(n.y) + u_sun * 0.8 * abs(dot(n, l));
  }} else {{
    k = u_amb + u_sky * n.y + u_sun * max(dot(n, l), 0.0);
  }}
  vec3 c = min(v_base * k, vec3(1.0));
  c = mix(c, v_hl.rgb, v_hl.a);
  // egui textures hold gamma-encoded values: multiply as they are (like the offscreen renderer)
  c *= t.rgb;
  {frag_out} = vec4(c, 1.0);
}}
"#
            );
            let program = gl.create_program()?;
            let mut shaders = vec![];
            for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
                let s = gl.create_shader(kind)?;
                gl.shader_source(s, &src);
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    return Err(gl.get_shader_info_log(s));
                }
                gl.attach_shader(program, s);
                shaders.push(s);
            }
            for (i, (name, _)) in ATTRS.iter().enumerate() {
                if !name.is_empty() {
                    gl.bind_attrib_location(program, i as u32, name);
                }
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            for s in shaders {
                gl.detach_shader(program, s);
                gl.delete_shader(s);
            }
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            let ebo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = (STRIDE * 4) as i32;
            let mut off = 0;
            for (i, (name, n)) in ATTRS.iter().enumerate() {
                if name.is_empty() {
                    continue;
                }
                gl.enable_vertex_attrib_array(i as u32);
                gl.vertex_attrib_pointer_f32(i as u32, *n, glow::FLOAT, false, stride, off * 4);
                off += n;
            }
            debug_assert_eq!(off as usize, STRIDE);
            gl.bind_vertex_array(None);
            let u = |n: &str| gl.get_uniform_location(program, n);
            Ok(Res {
                program,
                vao,
                vbo,
                ebo,
                u_tex: u("u_tex"),
                u_nrm: u("u_nrm"),
                u_use_tex: u("u_use_tex"),
                u_use_nrm: u("u_use_nrm"),
                u_green: u("u_green"),
                u_strength: u("u_strength"),
                u_light: u("u_light"),
                u_sun: u("u_sun"),
                u_amb: u("u_amb"),
                u_sky: u("u_sky"),
                u_unlit: u("u_unlit"),
                u_tex2: u("u_tex2"),
                u_nrm2: u("u_nrm2"),
                u_use_tex2: u("u_use_tex2"),
                u_use_nrm2: u("u_use_nrm2"),
                u_green2: u("u_green2"),
                u_strength2: u("u_strength2"),
                u_blend: u("u_blend"),
                u_ramp: u("u_ramp"),
            })
        }
    }

    unsafe fn draw(&self, gl: &glow::Context, painter: &egui_glow::Painter, info: &egui::PaintCallbackInfo, mesh: &GpuMesh) {
        if mesh.verts.is_empty() {
            return;
        }
        unsafe {
            let clip = info.clip_rect_in_pixels();
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(clip.left_px, clip.from_bottom_px, clip.width_px, clip.height_px);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes = std::slice::from_raw_parts(mesh.verts.as_ptr() as *const u8, mesh.verts.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ebo));
            gl.uniform_1_i32(self.u_tex.as_ref(), 0);
            gl.uniform_1_i32(self.u_nrm.as_ref(), 1);
            gl.uniform_1_i32(self.u_tex2.as_ref(), 2);
            gl.uniform_1_i32(self.u_nrm2.as_ref(), 3);
            gl.uniform_3_f32(self.u_light.as_ref(), mesh.light[0], mesh.light[1], mesh.light[2]);
            gl.uniform_3_f32(self.u_sun.as_ref(), mesh.sun[0], mesh.sun[1], mesh.sun[2]);
            gl.uniform_3_f32(self.u_amb.as_ref(), mesh.ambient[0], mesh.ambient[1], mesh.ambient[2]);
            gl.uniform_3_f32(self.u_sky.as_ref(), mesh.sky[0], mesh.sky[1], mesh.sky[2]);
            gl.uniform_1_f32(self.u_unlit.as_ref(), if mesh.unlit { 1.0 } else { 0.0 });
            // untextured draws still bind a valid texture (egui's own), so drivers stay quiet
            let fallback = painter.texture(egui::TextureId::default());
            for (key, idx) in &mesh.groups {
                if idx.is_empty() {
                    continue;
                }
                let col = key.color.and_then(|t| painter.texture(t));
                let nrm = key.normal.and_then(|t| painter.texture(t));
                gl.uniform_1_f32(self.u_use_tex.as_ref(), if col.is_some() { 1.0 } else { 0.0 });
                gl.uniform_1_f32(self.u_use_nrm.as_ref(), if nrm.is_some() { 1.0 } else { 0.0 });
                gl.uniform_1_f32(self.u_green.as_ref(), if key.flip_green { 1.0 } else { -1.0 });
                gl.uniform_1_f32(self.u_strength.as_ref(), key.strength_milli as f32 / 1000.0);
                let (col2, nrm2) = match key.blend {
                    Some(b) => (b.color.and_then(|t| painter.texture(t)), b.normal.and_then(|t| painter.texture(t))),
                    None => (None, None),
                };
                gl.uniform_1_f32(self.u_blend.as_ref(), if key.blend.is_some() { 1.0 } else { 0.0 });
                let r = key.blend.map(|b| [b.at, b.half, b.reach, b.scale].map(f32::from_bits)).unwrap_or([0.0; 4]);
                gl.uniform_4_f32(self.u_ramp.as_ref(), r[0], r[1], r[2], r[3]);
                gl.uniform_1_f32(self.u_use_tex2.as_ref(), if col2.is_some() { 1.0 } else { 0.0 });
                gl.uniform_1_f32(self.u_use_nrm2.as_ref(), if nrm2.is_some() { 1.0 } else { 0.0 });
                gl.uniform_1_f32(self.u_green2.as_ref(), if key.blend.map(|b| b.flip_green).unwrap_or(false) { 1.0 } else { -1.0 });
                gl.uniform_1_f32(self.u_strength2.as_ref(), key.blend.map(|b| b.strength_milli as f32 / 1000.0).unwrap_or(1.0));
                gl.active_texture(glow::TEXTURE3);
                gl.bind_texture(glow::TEXTURE_2D, nrm2.or(fallback));
                gl.active_texture(glow::TEXTURE2);
                gl.bind_texture(glow::TEXTURE_2D, col2.or(fallback));
                gl.active_texture(glow::TEXTURE1);
                gl.bind_texture(glow::TEXTURE_2D, nrm.or(fallback));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, col.or(fallback));
                let ib = std::slice::from_raw_parts(idx.as_ptr() as *const u8, idx.len() * 4);
                gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, ib, glow::STREAM_DRAW);
                gl.draw_elements(glow::TRIANGLES, idx.len() as i32, glow::UNSIGNED_INT, 0);
            }
            // hand the GL state back to egui
            gl.active_texture(glow::TEXTURE3);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(None);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
        }
    }
}
