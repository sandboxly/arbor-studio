//! Built-in Model Context Protocol server.
//!
//! * Streamable HTTP transport: `POST http://127.0.0.1:<port>/mcp` (JSON responses, no SSE).
//! * stdio transport: run the program with `--mcp-stdio`; it relays to the running window
//!   (and starts the window first if needed).

use crate::export::{self, Format};
use crate::grow::{build_mesh, grow};
use crate::model::*;
use crate::render::{render_offscreen, RenderMode};
use crate::state::{save_project, load_project, Shared, Source};
use base64::Engine;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Map, Value};
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 7878;
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Clone)]
pub struct Server {
    pub shared: Arc<Mutex<Shared>>,
    pub ctx: egui::Context,
    /// No window: window capture is unavailable.
    pub headless: bool,
}

pub fn start(server: Server, port: u16) -> Result<String, String> {
    let http = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| format!("port {port} is busy ({e})"))?;
    let real_port = http.server_addr().to_ip().map(|a| a.port()).unwrap_or(port);
    let url = format!("http://127.0.0.1:{real_port}/mcp");
    std::thread::Builder::new()
        .name("mcp-http".into())
        .spawn(move || {
            for req in http.incoming_requests() {
                let s = server.clone();
                std::thread::spawn(move || handle_http(&s, req));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(url)
}

fn header(req: &tiny_http::Request, name: &str) -> Option<String> {
    req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
}

fn respond(req: tiny_http::Request, code: u16, body: String, ctype: &str) {
    let mut r = tiny_http::Response::from_string(body).with_status_code(code);
    if let Ok(h) = tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes()) {
        r.add_header(h);
    }
    let _ = req.respond(r);
}

fn handle_http(s: &Server, mut req: tiny_http::Request) {
    // Reject cross-site browser requests (DNS-rebinding protection).
    if let Some(origin) = header(&req, "Origin") {
        let ok = origin == "null" || ["http://127.0.0.1", "http://localhost", "https://127.0.0.1", "https://localhost"].iter().any(|o| origin.starts_with(o));
        if !ok {
            return respond(req, 403, "forbidden origin".into(), "text/plain");
        }
    }
    let path = req.url().split('?').next().unwrap_or("").to_string();
    match (req.method(), path.as_str()) {
        (tiny_http::Method::Get, "/health") => respond(req, 200, "ok".into(), "text/plain"),
        (tiny_http::Method::Post, "/mcp") | (tiny_http::Method::Post, "/") => {
            let mut body = String::new();
            if req.as_reader().read_to_string(&mut body).is_err() {
                return respond(req, 400, "bad body".into(), "text/plain");
            }
            match handle_message(s, &body) {
                Some(out) => respond(req, 200, out, "application/json"),
                None => {
                    let _ = req.respond(tiny_http::Response::empty(202));
                }
            }
        }
        (tiny_http::Method::Get, "/mcp") | (tiny_http::Method::Delete, "/mcp") => {
            let mut r = tiny_http::Response::from_string("this server only answers POST").with_status_code(405);
            if let Ok(h) = tiny_http::Header::from_bytes(&b"Allow"[..], &b"POST"[..]) {
                r.add_header(h);
            }
            let _ = req.respond(r);
        }
        _ => respond(req, 404, "not found; MCP endpoint is /mcp".into(), "text/plain"),
    }
}

/// Handle one JSON-RPC message or batch. Returns None when nothing should be sent back.
pub fn handle_message(s: &Server, body: &str) -> Option<String> {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}).to_string()),
    };
    if let Value::Array(items) = v {
        let out: Vec<Value> = items.iter().filter_map(|m| handle_one(s, m)).collect();
        if out.is_empty() { None } else { Some(Value::Array(out).to_string()) }
    } else {
        handle_one(s, &v).map(|r| r.to_string())
    }
}

fn handle_one(s: &Server, m: &Value) -> Option<Value> {
    let id = m.get("id").cloned();
    let method = m.get("method").and_then(|x| x.as_str()).unwrap_or("");
    let params = m.get("params").cloned().unwrap_or(json!({}));
    id.as_ref()?; // notifications and responses get no reply
    let id = id.unwrap();
    let ok = |r: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": r}));
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
            let ver = if PROTOCOL_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSIONS[1] };
            if let Some(name) = params.pointer("/clientInfo/name").and_then(|v| v.as_str()) {
                let mut sh = s.shared.lock().unwrap();
                sh.mcp.client = Some(name.to_string());
                sh.log(Source::Agent, format!("{name} connected"));
                drop(sh);
                s.ctx.request_repaint();
            }
            ok(json!({
                "protocolVersion": ver,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "arbor-studio", "title": "ArborStudio tree modeller", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": tool_list()})),
        "resources/list" => ok(json!({"resources": []})),
        "resources/templates/list" => ok(json!({"resourceTemplates": []})),
        "prompts/list" => ok(json!({"prompts": []})),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let result = call_tool(s, &name, &args);
            {
                let mut sh = s.shared.lock().unwrap();
                sh.mcp.calls += 1;
                let short = summarize_args(&args);
                match &result {
                    Ok(_) => sh.log(Source::Agent, format!("{name}{short}")),
                    Err(e) => sh.log(Source::Agent, format!("{name}{short} → error: {e}")),
                }
            }
            s.ctx.request_repaint();
            match result {
                Ok(content) => ok(json!({"content": content, "isError": false})),
                Err(e) => ok(json!({"content": [{"type": "text", "text": e}], "isError": true})),
            }
        }
        _ => Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})),
    }
}

fn summarize_args(a: &Value) -> String {
    let s = a.to_string();
    if s == "{}" || s == "null" {
        return String::new();
    }
    let s: String = s.chars().take(90).collect();
    format!(" {s}")
}

const INSTRUCTIONS: &str = "ArborStudio is a procedural tree modeller. A tree is a hierarchy of generators: a trunk grows from the ground, roots spread from the trunk base along the ground, branches grow on trunks, roots or other branches, leaves grow on trunks or branches. Forces (direction pushes, attractor points) bend growth; a pruning volume trims the crown to a box or ellipsoid. LOD 4 is a billboard (the tree pictured from 8 sides on crossed cards) for far distances. Every change updates the open window live and can be undone. Typical flow: get_tree -> load_preset or update_generator/add_generator -> render_preview to look at the result -> export_tree (obj or glb for Unity/Unreal/Godot, json for custom engines). Call get_parameter_reference for parameter meanings and ranges.";

fn tool(name: &str, title: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "title": title, "description": desc, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

fn params_schema() -> Value {
    json!({
        "type": "object",
        "description": "Any subset of generator parameters; see get_parameter_reference.",
        "properties": {
            "count": {"type": "integer"}, "start": {"type": "number"}, "end": {"type": "number"},
            "phyllotaxis": {"type": "number"}, "whorl": {"type": "integer"}, "random": {"type": "number"},
            "length": {"type": "number"}, "length_variation": {"type": "number"},
            "profile": {"type": "string", "enum": ["conical", "spherical", "hemispherical", "cylindrical", "flame", "inverse_conical"]},
            "angle": {"type": "number"}, "angle_variation": {"type": "number"}, "gravity": {"type": "number"},
            "wiggle": {"type": "number"}, "segments": {"type": "integer"}, "radius": {"type": "number"},
            "taper": {"type": "number"}, "flare": {"type": "number"}, "sides": {"type": "integer"},
            "leaf_size": {"type": "number"}, "leaf_size_variation": {"type": "number"},
            "growth": {"type": "object", "description": "How this generator grows in the growth animation (see get_parameter_reference growth.*)", "properties": {
                "speed": {"type": "number", "minimum": 0.05, "maximum": 10},
                "speed_curve": {"type": "string", "enum": ["linear", "ease_in", "ease_out", "s_curve"]},
                "radius_curve": {"type": "string", "enum": ["linear", "ease_in", "ease_out", "s_curve"]},
                "start_size": {"type": "number", "minimum": 0, "maximum": 1},
                "start_angle": {"type": "number", "minimum": 0, "maximum": 1},
                "orientation_time": {"type": "number", "minimum": 0.1, "maximum": 5},
                "sync": {"type": "string", "enum": ["auto", "parent", "siblings", "parent_start_end", "in_place"]},
                "start_pct": {"type": "number", "minimum": 0, "maximum": 1},
                "offset": {"type": "number", "minimum": 0, "maximum": 3},
                "offset_variation": {"type": "number", "minimum": 0, "maximum": 3}
            }},
            "leaf_shape": {"type": "string", "enum": ["broad", "narrow", "needle", "round", "frond"]},
            "leaf_aspect": {"type": "number"}
        }
    })
}

pub fn tool_list() -> Vec<Value> {
    let color = json!({"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 3, "maxItems": 3});
    vec![
        tool("get_tree", "Get tree", "Return the whole tree: name, global settings, the generator hierarchy (ids, kinds, parents, all parameters), the selected generator, the view and mesh statistics.", json!({}), &[]),
        tool("get_parameter_reference", "Parameter reference", "Explain every generator parameter and global setting with ranges.", json!({}), &[]),
        tool("list_presets", "List presets", "List the built-in tree species presets.", json!({}), &[]),
        tool("load_preset", "Load preset", "Replace the current tree with a built-in preset (undoable).", json!({"name": {"type": "string", "description": "Preset name, e.g. Oak, Pine, Birch, Willow, Palm, Maple, Shrub, Sapling"}}), &["name"]),
        tool("new_tree", "New tree", "Start a fresh simple tree: trunk, branches and leaves with default settings.", json!({"name": {"type": "string"}}), &[]),
        tool("add_generator", "Add generator", "Add a generator. kind 'trunk' grows from the ground; 'branches' and 'leaves' need parent_id (a trunk or branches generator). Optional params set its parameters right away. Returns the new id.", json!({
            "kind": {"type": "string", "enum": ["trunk", "branches", "leaves", "roots"]},
            "parent_id": {"type": "integer"},
            "name": {"type": "string"},
            "material_id": {"type": ["integer", "null"], "description": "Material from the library (see list_materials); omit or null for the default look"},
            "params": params_schema()
        }), &["kind"]),
        tool("update_generator", "Update generator", "Change a generator's name, on/off state and/or any of its parameters (only the given ones change). Values are clamped to valid ranges.", json!({
            "id": {"type": "integer"},
            "name": {"type": "string"},
            "enabled": {"type": "boolean"},
            "material_id": {"type": ["integer", "null"], "description": "Material id to assign; null = default bark/leaf look"},
            "params": params_schema()
        }), &["id"]),
        tool("remove_generator", "Remove generator", "Delete a generator and everything that grows on it.", json!({"id": {"type": "integer"}}), &["id"]),
        tool("list_materials", "List materials", "List the tree's material library (ids, colours, textures, settings) and which generators use each one. Generators without a material use the default bark/leaf colours from the tree settings.", json!({}), &[]),
        tool("add_material", "Add material", "Create a material: a colour, optionally a colour texture (PNG/JPEG; the colour then tints it, use white [255,255,255] to show it unchanged) and a normal map. assign_to lists generator ids to use it right away. For leaf textures with transparency set alpha_cutout and give the leaves leaf_shape 'card'. Returns the new id.", material_schema(true), &["name"]),
        tool("update_material", "Update material", "Change any material fields (only the given ones change). Set texture or normal_map to null to remove the image.", material_schema(false), &["id"]),
        tool("remove_material", "Remove material", "Delete a material; generators that used it go back to the default look.", json!({"id": {"type": "integer"}}), &["id"]),
        tool("assign_material", "Assign material", "Give a generator a material from the library (material_id null = default look). Several generators can share one material.", json!({
            "generator_id": {"type": "integer"},
            "material_id": {"type": ["integer", "null"]}
        }), &["generator_id", "material_id"]),
        tool("update_settings", "Update tree settings", "Change global settings: name, seed, scale, bark_color, leaf_color, leaf_color_variation, autumn (0..1), leaf_drop (0..1, season: fallen leaves), ao_strength (0..1, baked ambient occlusion), age (0.05..1 growth stage) and wind {strength, direction, gustiness, speed}.", json!({
            "name": {"type": "string"},
            "seed": {"type": "integer"},
            "scale": {"type": "number"},
            "bark_color": color.clone(),
            "leaf_color": color,
            "leaf_color_variation": {"type": "number"},
            "autumn": {"type": "number"},
            "leaf_drop": {"type": "number", "minimum": 0, "maximum": 1},
            "ao_strength": {"type": "number", "minimum": 0, "maximum": 1},
            "age": {"type": "number", "minimum": 0.05, "maximum": 1, "description": "Growth stage (1 = mature)"},
            "join_branches": {"type": "boolean", "description": "Join branches to their parent (default true); join_method picks how"},
            "join_method": {"type": "string", "enum": ["stitch", "weld"], "description": "stitch = opening cut into the parent and stitched (one watertight mesh); weld = parent whole, branch base wrapped onto it with blended normals/colour/wind"},
            "branch_welding": {"type": "number", "minimum": 0, "maximum": 1, "description": "Set the weld (collar blending into the parent) of every branches generator at once; per level it is the generator param 'weld'"},
            "wind": {"type": "object", "properties": {"strength": {"type": "number"}, "direction": {"type": "number"}, "gustiness": {"type": "number"}, "speed": {"type": "number"}}}
        }), &[]),
        tool("randomize_seed", "Randomize", "Pick a new random seed: same species, different individual tree.", json!({}), &[]),
        tool("select_generator", "Select generator", "Select a generator in the window (it is highlighted in the viewport). Pass id null to clear.", json!({"id": {"type": ["integer", "null"]}}), &["id"]),
        tool("set_view", "Set view", "Change the viewport: camera (yaw, pitch degrees; distance metres; target [x,y,z]; fov), render mode, wind animation, LOD level 0-3, grid and leaf visibility. frame=true fits the tree in view.", json!({
            "yaw": {"type": "number"}, "pitch": {"type": "number"}, "distance": {"type": "number"},
            "target": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "fov": {"type": "number"},
            "mode": {"type": "string", "enum": ["shaded", "wireframe", "skeleton"]},
            "wind_playing": {"type": "boolean"}, "lod": {"type": "integer", "minimum": 0, "maximum": 4, "description": "0-3 = full to reduced geometry, 4 = billboard"},
            "growth": {"type": "number", "minimum": 0, "maximum": 1, "description": "Preview the growth animation at this point (0 = nothing above ground, 1 = the mature tree)"},
            "growth_stage": {"type": "string", "description": "Preview the growth at this stage (by name, see set_growth)"},
            "growth_playing": {"type": "boolean", "description": "Play the growth in the window (from the start when at the mature tree)"},
            "show_grid": {"type": "boolean"}, "show_leaves": {"type": "boolean"}, "frame": {"type": "boolean"},
            "show_helpers": {"type": "boolean", "description": "Draw forces and the pruning volume"},
            "show_collision": {"type": "boolean", "description": "Draw the collision capsules"},
            "highlight_selection": {"type": "boolean", "description": "Tint the selected generator in the viewport"},
            "lighting": {"type": "string", "enum": ["unlit", "morning", "noon", "evening", "night"], "description": "Viewport lighting preset (also used by render_preview)"}
        }), &[]),
        tool("get_stats", "Statistics", "Stem, leaf, vertex and triangle counts plus height and crown width, for one LOD level or all.", json!({"lod": {"type": "integer", "minimum": 0, "maximum": 4}, "all_lods": {"type": "boolean"}}), &[]),
        tool("render_preview", "Render preview", "Render the tree to a PNG image (works even when the window is hidden). The tree is fitted in view from the window's yaw/pitch unless use_window_camera is true; yaw/pitch/distance override.", json!({
            "use_window_camera": {"type": "boolean"},
            "lighting": {"type": "string", "enum": ["unlit", "morning", "noon", "evening", "night"], "description": "Lighting preset; default: the window's"},
            "target": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "Point to look at in metres, e.g. [0,0.3,0] for a close look at the roots"},
            "width": {"type": "integer", "minimum": 64, "maximum": 2048}, "height": {"type": "integer", "minimum": 64, "maximum": 2048},
            "yaw": {"type": "number"}, "pitch": {"type": "number"}, "distance": {"type": "number"},
            "lod": {"type": "integer", "minimum": 0, "maximum": 4}, "save_to": {"type": "string", "description": "Optional absolute path to also save the PNG"},
            "growth": {"type": "number", "minimum": 0, "maximum": 1, "description": "Render the tree at this point of its growth (0 = nothing above ground, 1 = mature; default 1). The camera still frames the mature tree unless distance/target are given."},
            "growth_stage": {"type": "string", "description": "Render the tree at this growth stage (by name, see set_growth)"},
            "wind_time": {"type": "number", "description": "Render the tree bent by the wind at this time in seconds, with the tree's wind settings (as the viewport plays it); render a series of times for an animation. Default: no wind."}
        }), &[]),
        tool("capture_window", "Capture window", "Take a picture of the whole application window (panels and viewport) as PNG. The window must be open and visible.", json!({"save_to": {"type": "string"}}), &[]),
        tool("export_tree", "Export tree", "Export the tree for a game engine or DCC tool. format obj (+mtl) or glb work in Unity, Unreal and Godot; json is the raw skeleton + settings for custom engines. include_lods adds LOD0-3 meshes named <Tree>_LOD0..3. include_billboard adds a billboard LOD (LOD4, or <Tree>_Billboard without LODs) and saves its atlas picture as <name>_billboard.png next to the file. Wind sway weights are stored in UV2 (TEXCOORD_1) in glb.", json!({
            "path": {"type": "string", "description": "Absolute output path, e.g. /Users/me/Game/Assets/Trees/oak.glb"},
            "format": {"type": "string", "enum": ["obj", "glb", "json"]},
            "include_lods": {"type": "boolean"},
            "include_billboard": {"type": "boolean"},
            "include_growth": {"type": "boolean", "description": "Add the growth animation (default: the tree's growth.export setting). glb: morph targets with a Grow clip and one clip per stage; obj: one extra file per stage (<name>_<Stage>.obj); json: growth timing per stem and leaf. See set_growth."}
        }), &["path"]),
        tool("export_variations", "Export variations", "Export several individuals of the current species (different seeds) into a folder, one file each: <Tree>_v01.glb, <Tree>_v02.glb, ... Mesh names carry the same suffix. Node edits are not applied to variations.", json!({
            "folder": {"type": "string", "description": "Absolute output folder"},
            "count": {"type": "integer", "minimum": 1, "maximum": 64},
            "first_seed": {"type": "integer", "description": "Seed of the first variation (default: the current seed)"},
            "format": {"type": "string", "enum": ["obj", "glb", "json"]},
            "include_lods": {"type": "boolean"},
            "include_billboard": {"type": "boolean"},
            "include_growth": {"type": "boolean", "description": "Add the growth animation to every file (default: the tree's growth.export setting)"}
        }), &["folder", "count"]),
        tool("add_force", "Add force", "Add a force that bends stems while they grow. kind 'direction' pushes every stem along direction [x,y,z] (wind-swept or leaning trees); kind 'attractor' pulls stems towards position [x,y,z] in metres (light seeking), negative strength pushes them away (obstacles). Each generator's force_response parameter scales the effect (roots default to 0).", json!({
            "kind": {"type": "string", "enum": ["direction", "attractor"]},
            "name": {"type": "string"},
            "direction": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "position": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "strength": {"type": "number", "minimum": -2, "maximum": 2},
            "enabled": {"type": "boolean"}
        }), &["kind"]),
        tool("update_force", "Update force", "Change a force (only the given fields change).", json!({
            "id": {"type": "integer"},
            "kind": {"type": "string", "enum": ["direction", "attractor"]},
            "name": {"type": "string"},
            "direction": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "position": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "strength": {"type": "number", "minimum": -2, "maximum": 2},
            "enabled": {"type": "boolean"}
        }), &["id"]),
        tool("remove_force", "Remove force", "Delete a force.", json!({"id": {"type": "integer"}}), &["id"]),
        tool("set_curve", "Set curve", "Shape a generator value with a curve (like the original's per-property curves). The value becomes value x curve(x), where x runs along the parent (within start..end) or along the stem itself, depending on the curve (see get_parameter_reference -> curves), or with along=tree_height up the whole tree (0 = ground, 1 = top). Give points [[x,y],...] with x and y in 0..1, or a preset (flat, ramp_up, ramp_down, bell, valley, ease_out), or points null to remove the curve. Example: curve taper, preset ramp_up = weak taper at the base, strong towards the tip. Giving only `along` changes the axis of an existing curve.", json!({
            "generator_id": {"type": "integer"},
            "curve": {"type": "string", "enum": ["length", "angle", "radius", "density", "leaf_size", "radius_profile", "taper", "gravity", "wiggle", "jitter", "force_response", "flute_depth", "flatten", "bumps", "angle_variation", "length_variation", "kink", "break_chance", "leaf_fold", "leaf_bend", "leaf_face_up"]},
            "along": {"type": "string", "enum": ["natural", "tree_height"], "description": "natural = along the stem or the parent (the curve's own axis); tree_height = height above the ground as a fraction of the tree"},
            "points": {"type": ["array", "null"], "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}},
            "preset": {"type": "string", "enum": ["flat", "ramp_up", "ramp_down", "bell", "valley", "ease_out"]}
        }), &["generator_id", "curve"]),
        tool("list_nodes", "List branch nodes", "List individual branch instances (nodes) with their stable keys, so single branches can be hand-edited with edit_node. Filter by generator_id; at most `limit` (default 200) are returned. Leaves are not nodes.", json!({
            "generator_id": {"type": "integer"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 2000}
        }), &[]),
        tool("edit_node", "Edit branch node", "Hand-edit one branch instance (node editing): delete it (hidden), scale its length or thickness, add to its angle from the parent or its rotation around the parent. Edits are kept by key; they reset if counts above the branch change.", json!({
            "key": {"type": "integer"},
            "hidden": {"type": "boolean"},
            "length": {"type": "number", "minimum": 0.05, "maximum": 3, "description": "Length multiplier"},
            "radius": {"type": "number", "minimum": 0.1, "maximum": 3, "description": "Thickness multiplier"},
            "angle": {"type": "number", "minimum": -120, "maximum": 120, "description": "Degrees added to the angle from the parent"},
            "rotate": {"type": "number", "minimum": -180, "maximum": 180, "description": "Degrees added around the parent"},
            "reset": {"type": "boolean", "description": "Remove all edits of this node"}
        }), &["key"]),
        tool("clear_node_edits", "Clear node edits", "Remove all hand edits of individual branches (restores deleted branches).", json!({}), &[]),
        tool("set_collision", "Set collision", "Collision objects for game physics: capsules fitted to the trunk and thick branches, exported as convex hulls (glb/obj) and capsule data (glb node extras, json). naming picks the engine convention: neutral (<Tree>_Collision_NN), unreal (UCX_<mesh>_NN) or godot (-convcolonly suffix). Returns the capsules.", json!({
            "enabled": {"type": "boolean"},
            "max_depth": {"type": "integer", "minimum": 0, "maximum": 4, "description": "0 = trunks only, 1 = plus main branches, ..."},
            "min_radius": {"type": "number", "description": "Thinnest stem (m) that gets collision"},
            "segments": {"type": "integer", "minimum": 1, "maximum": 12, "description": "Capsules per stem"},
            "radius_scale": {"type": "number", "minimum": 0.2, "maximum": 3},
            "naming": {"type": "string", "enum": ["neutral", "unreal", "godot"]},
            "show": {"type": "boolean", "description": "Draw the capsules in the window"}
        }), &[]),
        tool("set_triangle_budget", "Set triangle budget", "Triangle budgets for the mesh levels of detail LOD0 to LOD3 (the billboard, LOD4, is always 16 triangles). Give max to derive all four from one maximum (LOD0 = max, LOD1 = max/2, LOD2 = max/4, LOD3 = max/8), or levels to set each one (0 = no limit). With fit on (default), a level over its budget gets fewer sides, rings and larger leaves until it fits; with fit off, levels keep their usual detail and are only reported. Only the given fields change; giving max or levels turns budgets on. Returns the budget and the check of every level, like check_triangle_budget.", json!({
            "enabled": {"type": "boolean"},
            "max": {"type": "integer", "minimum": 16, "maximum": 5000000, "description": "Maximum for LOD0; the other levels are derived from it"},
            "levels": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 5000000}, "minItems": 4, "maxItems": 4, "description": "Budgets for LOD0..LOD3 (0 = no limit)"},
            "auto": {"type": "boolean", "description": "true: derive from max; false: use levels (set automatically when max or levels is given)"},
            "fit": {"type": "boolean", "description": "Reduce detail until each level fits (default true)"}
        }), &[]),
        tool("check_triangle_budget", "Check triangle budget", "Check the current tree against its triangle budgets: for each mesh level (LOD0..LOD3, or only `lod`) the budget, the triangles it has, the triangles at its usual detail, whether detail was reduced to fit and whether it is over budget. within_budget is true when no level is over. Works when budgets are off too (then there is no budget to be over).", json!({
            "lod": {"type": "integer", "minimum": 0, "maximum": 3, "description": "Check one level only"}
        }), &[]),
        tool("set_growth", "Set growth", "The growth animation: the tree grows from a seedling to the mature tree on one mesh (the whole tree grows at once: a young tree is a small, slender tree with its branches, they come out of closed bark, and leaves keep coming so the crown gets denser), so engines play it as morph targets. Stages are named moments of the growth, from 0 (nothing above ground) to 1 (the mature tree); exports get a Grow clip and one clip per stage (first stage: from nothing; then '<A> to <B>'). Only the given fields change. Returns the settings, the keyframes and the clips with their times. Preview with render_preview growth/growth_stage or set_view.", json!({
            "stages": {"type": "array", "maxItems": 8, "items": {"type": "object", "properties": {
                "name": {"type": "string", "description": "Optional; unnamed stages are called 'Stage <n>' by their place"},
                "at": {"type": "number", "minimum": 0.01, "maximum": 1}
            }, "required": ["at"]}, "description": "The stages in growing order; replaces all stages. E.g. [{name: Sprout, at: 0.12}, {name: Sapling, at: 0.38}, {at: 0.68}, {name: Mature, at: 1}]. How each part grows is set per generator: update_generator params.growth."},
            "duration": {"type": "number", "minimum": 0.5, "maximum": 600, "description": "Seconds the whole growth takes in the exported animation"},
            "keyframes": {"type": "integer", "minimum": 4, "maximum": 48, "description": "Shapes the growth is sampled at (morph targets); stages always get one"},
            "export": {"type": "boolean", "description": "Exports include the growth animation by default"}
        }), &[]),
        tool("set_pruning", "Set pruning", "Trim branches and leaves to a volume (topiary, hedges, fitting a tree into a space). Branches stop at the surface; trunks are only cut at the top; roots are never pruned. fit_to_tree places a volume slightly smaller than the current crown.", json!({
            "enabled": {"type": "boolean"},
            "shape": {"type": "string", "enum": ["box", "ellipsoid"]},
            "center": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "size": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "Half-size along x, y, z in metres"},
            "fit_to_tree": {"type": "boolean"}
        }), &[]),
        tool("save_project", "Save project", "Save the editable tree project (.arbor.json). Without path, saves to the current file.", json!({"path": {"type": "string"}}), &[]),
        tool("open_project", "Open project", "Open a saved tree project (.arbor.json).", json!({"path": {"type": "string"}}), &["path"]),
        tool("undo", "Undo", "Undo the last change.", json!({}), &[]),
        tool("redo", "Redo", "Redo the last undone change.", json!({}), &[]),
    ]
}

fn text(v: impl Into<String>) -> Vec<Value> {
    vec![json!({"type": "text", "text": v.into()})]
}

fn pretty(v: &Value) -> Vec<Value> {
    text(serde_json::to_string_pretty(&tidy(v.clone())).unwrap())
}

/// Round floats to 4 decimals so f32 values read cleanly (0.1 instead of 0.10000000149).
fn tidy(v: Value) -> Value {
    match v {
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap();
            json!((f * 10000.0).round() / 10000.0)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(tidy).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, tidy(v))).collect()),
        other => other,
    }
}

fn image(png: Vec<u8>, note: String) -> Vec<Value> {
    vec![
        json!({"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(png), "mimeType": "image/png"}),
        json!({"type": "text", "text": note}),
    ]
}

/// Merge a JSON object patch into a serde value, refusing unknown keys.
pub fn merge_into<T: Serialize + DeserializeOwned>(target: &mut T, patch: &Value, what: &str) -> Result<(), String> {
    let mut v = serde_json::to_value(&*target).map_err(|e| e.to_string())?;
    merge_value(&mut v, patch, what)?;
    *target = serde_json::from_value(v).map_err(|e| format!("invalid {what}: {e}"))?;
    Ok(())
}

fn merge_value(v: &mut Value, patch: &Value, what: &str) -> Result<(), String> {
    let (Some(obj), Some(p)) = (v.as_object_mut(), patch.as_object()) else {
        return Err(format!("{what} must be an object"));
    };
    for (k, val) in p {
        let Some(cur) = obj.get_mut(k) else {
            let keys: Vec<&String> = obj.keys().collect();
            return Err(format!("unknown {what} field '{k}'. Valid fields: {keys:?}"));
        };
        if cur.is_object() && val.is_object() && k != "curves" {
            merge_value(cur, val, what)?;
        } else if cur.is_u64() && val.is_f64() {
            *cur = json!(val.as_f64().unwrap().round().max(0.0) as u64);
        } else if val.is_string() && (k == "profile" || k == "leaf_shape") {
            *cur = json!(val.as_str().unwrap().to_ascii_lowercase().replace([' ', '-'], "_"));
        } else {
            *cur = val.clone();
        }
    }
    Ok(())
}

fn arg_u32(a: &Value, k: &str) -> Option<u32> {
    a.get(k).and_then(|v| v.as_f64()).map(|f| f.max(0.0) as u32)
}
fn arg_f32(a: &Value, k: &str) -> Option<f32> {
    a.get(k).and_then(|v| v.as_f64()).map(|f| f as f32)
}
fn arg_str<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(|v| v.as_str())
}

/// A growth point from `growth` (0..1) or `growth_stage` (a stage name).
fn growth_arg(a: &Value, g: &crate::model::Growth) -> Result<Option<f32>, String> {
    if let Some(name) = arg_str(a, "growth_stage") {
        let labels: Vec<String> = (0..g.stages.len()).map(|i| g.label(i)).collect();
        let i = labels.iter().position(|l| l.eq_ignore_ascii_case(name.trim())).ok_or_else(|| format!("no growth stage named {name}; stages: {}", labels.join(", ")))?;
        return Ok(Some(g.stages[i].at));
    }
    Ok(arg_f32(a, "growth").map(|x| x.clamp(0.0, 1.0)))
}

/// The growth settings with their keyframes and clips (times in seconds).
pub fn growth_info(g: &crate::model::Growth) -> Value {
    let keys = crate::growth::keyframes(g);
    let clips: Vec<Value> = crate::growth::clips(g)
        .iter()
        .map(|(n, a, b)| json!({"name": n, "from": a, "to": b, "seconds": ((b - a) * g.duration * 100.0).round() / 100.0}))
        .collect();
    json!({
        "growth": g,
        "keyframes": keys,
        "morph_targets": keys.len() - 1,
        "clips": clips,
    })
}

/// The tree's levels against their triangle budgets, for set/check_triangle_budget.
fn budget_check(doc: &Document, lod: Option<u32>) -> Value {
    let levels: Vec<crate::budget::LevelFit> = match lod {
        Some(l) => vec![crate::budget::level_mesh(doc, l).2],
        None => crate::budget::report(doc),
    };
    let rows: Vec<Value> = levels
        .iter()
        .map(|f| {
            json!({
                "lod": f.lod, "budget": f.budget, "triangles": f.triangles, "usual_triangles": f.usual_triangles,
                "reduced": f.reduced, "over": f.over, "reduction": f.reduction, "message": f.message(doc.budget.fit)
            })
        })
        .collect();
    json!({
        "budget": doc.budget,
        "budgets_in_effect": doc.budget.enabled.then(|| doc.budget.derived()),
        "within_budget": levels.iter().all(|f| !f.over),
        "levels": rows,
        "billboard": "LOD4 is always 16 triangles and has no budget"
    })
}

fn stats_for(doc: &Document, lod: u32) -> Value {
    let l = crate::billboard::lod_mesh(doc, lod);
    json!(l.mesh.stats(&l.skeleton))
}

fn gen_json(doc: &Document, g: &Generator) -> Value {
    let children: Vec<u32> = doc.children(Some(g.id)).map(|c| c.id).collect();
    let material = g.material.and_then(|m| doc.material(m)).map(|m| m.name.clone()).unwrap_or_else(|| if g.kind == GenKind::Leaves { "Default leaves".into() } else { "Default bark".into() });
    json!({"id": g.id, "name": g.name, "kind": g.kind, "parent_id": g.parent, "enabled": g.enabled, "material_id": g.material, "material": material, "children": children, "params": g.params})
}

fn tree_json(sh: &Shared) -> Value {
    let doc = &sh.doc;
    let gens: Vec<Value> = doc.ordered().iter().filter_map(|(id, _)| doc.get(*id)).map(|g| gen_json(doc, g)).collect();
    json!({
        "name": doc.name,
        "file": sh.file.as_ref().map(|p| p.display().to_string()),
        "unsaved_changes": sh.dirty,
        "settings": doc.settings,
        "materials": materials_json(doc),
        "forces": doc.forces,
        "pruning": doc.pruning,
        "collision": doc.collision,
        "node_edits": doc.nodes,
        "generators": gens,
        "selected_id": sh.selected,
        "view": sh.view,
        "can_undo": !sh.undo.is_empty() || sh.doc != sh.committed,
        "can_redo": !sh.redo.is_empty(),
    })
}

fn expand(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

pub fn call_tool(s: &Server, name: &str, a: &Value) -> Result<Vec<Value>, String> {
    let lock = || s.shared.lock().unwrap();
    match name {
        "get_tree" => {
            let (mut v, doc, lod) = {
                let sh = lock();
                (tree_json(&sh), sh.doc.clone(), sh.view.lod)
            };
            v["stats"] = stats_for(&doc, lod);
            Ok(pretty(&v))
        }
        "get_parameter_reference" => {
            let params: Vec<Value> = PARAM_REFERENCE.iter().map(|(f, applies, range, m)| json!({"field": f, "applies_to": applies, "range": range, "meaning": m})).collect();
            let settings: Vec<Value> = SETTINGS_REFERENCE.iter().map(|(f, range, m)| json!({"field": f, "range": range, "meaning": m})).collect();
            let materials: Vec<Value> = MATERIAL_REFERENCE.iter().map(|(f, range, m)| json!({"field": f, "range": range, "meaning": m})).collect();
            let curves: Vec<Value> = CURVE_REFERENCE.iter().map(|(k, along, applies, m)| json!({"curve": k, "along": along, "applies_to": applies, "meaning": m})).collect();
            Ok(pretty(&json!({"generator_params": params, "settings": settings, "material_fields": materials, "curves": curves,
                "hierarchy": "trunk (from ground) -> roots (on a trunk; follow the ground) and branches (on trunk, roots or branches, any depth) -> leaves (on trunk or branches). count is per parent stem, so totals multiply down the hierarchy. For roots, start/end are heights on the trunk (keep near 0) and angle is measured from the trunk's upward direction (90 = horizontal, >90 = downward)."})))
        }
        "list_presets" => Ok(pretty(&json!(PRESETS.iter().map(|(n, d)| json!({"name": n, "description": d})).collect::<Vec<_>>()))),
        "load_preset" => {
            let n = arg_str(a, "name").ok_or("name is required")?;
            let doc = preset(n).ok_or_else(|| format!("unknown preset '{n}'. Available: {:?}", preset_names()))?;
            let mut sh = lock();
            sh.replace_doc(doc);
            Ok(pretty(&tree_json(&sh)))
        }
        "new_tree" => {
            let mut doc = Document { name: arg_str(a, "name").unwrap_or("New tree").to_string(), settings: TreeSettings::default(), generators: vec![], next_id: 1, materials: vec![], forces: vec![], pruning: Pruning::default(), nodes: vec![], collision: Default::default(), budget: Default::default(), growth: Default::default() };
            let t = doc.add(GenKind::Trunk, None, Some("Trunk".into()))?;
            let b = doc.add(GenKind::Branches, Some(t), Some("Branches".into()))?;
            doc.add(GenKind::Leaves, Some(b), Some("Leaves".into()))?;
            let mut sh = lock();
            sh.replace_doc(doc);
            Ok(pretty(&tree_json(&sh)))
        }
        "add_generator" => {
            let kind = GenKind::parse(arg_str(a, "kind").ok_or("kind is required")?).ok_or("kind must be trunk, branches, leaves or roots")?;
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let id = doc.add(kind, arg_u32(a, "parent_id"), arg_str(a, "name").map(String::from))?;
            if let Some(p) = a.get("params").filter(|p| !p.is_null()) {
                let g = doc.get_mut(id).unwrap();
                merge_into(&mut g.params, p, "params")?;
                g.params.clamp(kind);
            }
            if let Some(mv) = a.get("material_id") {
                let mid = material_arg(&doc, mv)?;
                doc.get_mut(id).unwrap().material = mid;
            }
            sh.doc = doc;
            sh.selected = Some(id);
            sh.commit();
            let g = gen_json(&sh.doc, sh.doc.get(id).unwrap());
            Ok(pretty(&json!({"added": g})))
        }
        "update_generator" => {
            let id = arg_u32(a, "id").ok_or("id is required")?;
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let g = doc.get_mut(id).ok_or_else(|| format!("no generator with id {id}"))?;
            if let Some(n) = arg_str(a, "name") {
                g.name = n.to_string();
            }
            if let Some(e) = a.get("enabled").and_then(|v| v.as_bool()) {
                g.enabled = e;
            }
            if let Some(mv) = a.get("material_id") {
                let mid = material_arg(&sh.doc, mv)?;
                g.material = mid;
            }
            if let Some(p) = a.get("params").filter(|p| !p.is_null()) {
                merge_into(&mut g.params, p, "params")?;
            }
            let k = g.kind;
            g.params.clamp(k);
            sh.doc = doc;
            sh.commit();
            let g = gen_json(&sh.doc, sh.doc.get(id).unwrap());
            Ok(pretty(&json!({"updated": g})))
        }
        "add_force" => {
            let kind = force_kind(a)?.ok_or("kind is required (direction or attractor)")?;
            let mut f = Force { kind, ..Default::default() };
            if kind == ForceKind::Attractor {
                f.strength = 0.6;
            }
            apply_force_patch(&mut f, a)?;
            let mut sh = lock();
            let id = sh.doc.add_force(f);
            sh.commit();
            let f = sh.doc.forces.iter().find(|f| f.id == id).cloned();
            Ok(pretty(&json!({"added": f})))
        }
        "update_force" => {
            let id = arg_u32(a, "id").ok_or("id is required")?;
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let f = doc.force_mut(id).ok_or_else(|| format!("no force with id {id}"))?;
            if let Some(k) = force_kind(a)? {
                f.kind = k;
            }
            apply_force_patch(f, a)?;
            sh.doc = doc;
            sh.commit();
            let f = sh.doc.forces.iter().find(|f| f.id == id).cloned();
            Ok(pretty(&json!({"updated": f})))
        }
        "set_curve" => {
            let gid = arg_u32(a, "generator_id").ok_or("generator_id is required")?;
            let key = arg_str(a, "curve").ok_or("curve is required")?.to_string();
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let g = doc.get_mut(gid).ok_or_else(|| format!("no generator with id {gid}"))?;
            if !curve_applies(&key, g.kind) {
                let ok: Vec<&str> = CURVE_REFERENCE.iter().filter(|c| curve_applies(c.0, g.kind)).map(|c| c.0).collect();
                return Err(format!("curve '{key}' does not apply to {} generators; available: {ok:?}", g.kind.label()));
            }
            let along = match arg_str(a, "along") {
                None => None,
                Some("natural") => Some(CurveAlong::Natural),
                Some("tree_height") | Some("height") => Some(CurveAlong::TreeHeight),
                Some(o) => return Err(format!("along must be natural or tree_height, not '{o}'")),
            };
            let only_along = along.is_some() && arg_str(a, "preset").is_none() && a.get("points").is_none();
            let curve = if only_along {
                Some(g.params.curves.get(&key).cloned().ok_or_else(|| format!("generator {gid} has no '{key}' curve yet; give points or a preset"))?)
            } else if let Some(pr) = arg_str(a, "preset") {
                Some(Curve::preset(pr).ok_or_else(|| format!("unknown preset '{pr}'; available: {:?}", Curve::PRESETS))?)
            } else {
                match a.get("points") {
                    None | Some(Value::Null) => None,
                    Some(v) => {
                        let pts: Vec<[f32; 2]> = serde_json::from_value(v.clone()).map_err(|e| format!("points must be [[x,y],...]: {e}"))?;
                        let mut c = Curve { points: pts, along: CurveAlong::Natural };
                        c.clamp();
                        Some(c)
                    }
                }
            };
            match curve {
                Some(mut c) => {
                    // keep the existing axis unless a new one is given
                    c.along = along.or_else(|| g.params.curves.get(&key).map(|o| o.along)).unwrap_or_default();
                    g.params.curves.insert(key.clone(), c);
                }
                None => {
                    g.params.curves.remove(&key);
                }
            }
            let out = json!({"generator_id": gid, "curves": g.params.curves});
            sh.doc = doc;
            sh.commit();
            Ok(pretty(&out))
        }
        "list_nodes" => {
            let doc = lock().doc.clone();
            let sk = grow(&doc, 0);
            let filter = arg_u32(a, "generator_id");
            let limit = arg_u32(a, "limit").unwrap_or(200).clamp(1, 2000) as usize;
            let matching: Vec<&crate::grow::Stem> = sk.stems.iter().filter(|s| filter.map(|f| s.gen == f).unwrap_or(true)).collect();
            let nodes: Vec<Value> = matching
                .iter()
                .take(limit)
                .map(|s| {
                    let parent_key = s.parent.map(|p| sk.stems[p].key);
                    let base = s.points[0];
                    let tip = *s.points.last().unwrap();
                    json!({"key": s.key, "generator_id": s.gen, "generator": doc.get(s.gen).map(|g| g.name.clone()), "depth": s.depth,
                        "parent_key": parent_key, "base": [base.x, base.y, base.z], "tip": [tip.x, tip.y, tip.z], "length_m": s.length,
                        "edited": doc.node(s.key).is_some()})
                })
                .collect();
            let hidden: Vec<u32> = doc.nodes.iter().filter(|n| n.hidden).map(|n| n.key).collect();
            Ok(pretty(&json!({"total": matching.len(), "returned": nodes.len(), "nodes": nodes, "deleted_keys": hidden})))
        }
        "edit_node" => {
            let key = a.get("key").and_then(|v| v.as_u64()).ok_or("key is required")? as u32;
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            if a.get("reset").and_then(|v| v.as_bool()).unwrap_or(false) {
                doc.nodes.retain(|n| n.key != key);
            } else {
                let exists = doc.node(key).is_some() || grow(&doc, 0).stems.iter().any(|s| s.key == key);
                if !exists {
                    return Err(format!("no branch with key {key}; use list_nodes"));
                }
                let n = doc.node_mut(key);
                let mut patch = Map::new();
                for k in ["hidden", "length", "radius", "angle", "rotate"] {
                    if let Some(v) = a.get(k) {
                        patch.insert(k.into(), v.clone());
                    }
                }
                merge_into(n, &Value::Object(patch), "node")?;
                n.clamp();
            }
            doc.tidy_nodes();
            let out = json!({"key": key, "edit": doc.node(key)});
            sh.doc = doc;
            sh.commit();
            Ok(pretty(&out))
        }
        "clear_node_edits" => {
            let mut sh = lock();
            let n = sh.doc.nodes.len();
            sh.doc.nodes.clear();
            sh.commit();
            Ok(text(format!("cleared {n} node edit(s)")))
        }
        "remove_force" => {
            let id = arg_u32(a, "id").ok_or("id is required")?;
            let mut sh = lock();
            if !sh.doc.remove_force(id) {
                return Err(format!("no force with id {id}"));
            }
            sh.commit();
            Ok(text(format!("removed force {id}")))
        }
        "set_triangle_budget" => {
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let b = &mut doc.budget;
            if let Some(v) = a.get("max").and_then(|v| v.as_u64()) {
                b.max = v.min(u32::MAX as u64) as u32;
                b.auto = true;
                b.enabled = true;
            }
            if let Some(v) = a.get("levels").and_then(|v| v.as_array()) {
                let l: Vec<u32> = v.iter().filter_map(|x| x.as_u64()).map(|x| x.min(u32::MAX as u64) as u32).collect();
                if l.len() != 4 {
                    return Err("levels needs four numbers: LOD0, LOD1, LOD2, LOD3".into());
                }
                b.levels.copy_from_slice(&l);
                b.auto = false;
                b.enabled = true;
            }
            if let Some(v) = a.get("auto").and_then(|v| v.as_bool()) {
                b.auto = v;
            }
            if let Some(v) = a.get("fit").and_then(|v| v.as_bool()) {
                b.fit = v;
            }
            if let Some(v) = a.get("enabled").and_then(|v| v.as_bool()) {
                b.enabled = v;
            }
            b.clamp();
            sh.doc = doc.clone();
            sh.commit();
            drop(sh);
            Ok(pretty(&budget_check(&doc, None)))
        }
        "set_growth" => {
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let g = &mut doc.growth;
            if let Some(list) = a.get("stages") {
                let list = list.as_array().ok_or("stages must be a list of {name, at, generators}")?;
                let mut stages = vec![];
                for st in list {
                    let name = st.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let at = st.get("at").and_then(|v| v.as_f64()).ok_or("every stage needs at (0.01..1)")? as f32;
                    let stage = crate::model::GrowthStage::named(name, at);
                    stages.push(stage);
                }
                if stages.is_empty() {
                    return Err("give at least one stage".into());
                }
                g.stages = stages;
            }
            if let Some(v) = arg_f32(a, "duration") {
                g.duration = v;
            }
            if let Some(v) = arg_u32(a, "keyframes") {
                g.keyframes = v;
            }
            if let Some(v) = a.get("export").and_then(|v| v.as_bool()) {
                g.export = v;
            }
            doc.clamp_all();
            sh.doc = doc.clone();
            sh.commit();
            drop(sh);
            Ok(pretty(&growth_info(&doc.growth)))
        }
        "check_triangle_budget" => {
            let doc = lock().doc.clone();
            let lod = arg_u32(a, "lod").map(|l| l.min(3));
            Ok(pretty(&budget_check(&doc, lod)))
        }
        "set_collision" => {
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let mut patch = Map::new();
            for k in ["enabled", "max_depth", "min_radius", "segments", "radius_scale", "naming"] {
                if let Some(v) = a.get(k) {
                    let v = if k == "naming" { json!(v.as_str().unwrap_or("").to_ascii_lowercase()) } else { v.clone() };
                    patch.insert(k.into(), v);
                }
            }
            merge_into(&mut doc.collision, &Value::Object(patch), "collision")?;
            doc.collision.clamp();
            if let Some(s) = a.get("show").and_then(|v| v.as_bool()) {
                sh.view.show_collision = s;
            }
            let caps = crate::collision::capsules(&doc, &grow(&doc, 0));
            sh.doc = doc;
            sh.commit();
            Ok(pretty(&json!({"collision": sh.doc.collision, "capsules": caps})))
        }
        "set_pruning" => {
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            if a.get("fit_to_tree").and_then(|v| v.as_bool()).unwrap_or(false) {
                let mut free = doc.clone();
                free.pruning.enabled = false;
                let m = build_mesh(&free, &grow(&free, 0), 0);
                doc.pruning.fit(m.height, m.radius);
                doc.pruning.enabled = true;
            }
            let mut patch = Map::new();
            for k in ["enabled", "shape", "center", "size"] {
                if let Some(v) = a.get(k) {
                    let v = if k == "shape" { json!(v.as_str().unwrap_or("").to_ascii_lowercase()) } else { v.clone() };
                    patch.insert(k.into(), v);
                }
            }
            merge_into(&mut doc.pruning, &Value::Object(patch), "pruning")?;
            doc.pruning.clamp();
            sh.doc = doc;
            sh.commit();
            Ok(pretty(&json!({"pruning": sh.doc.pruning})))
        }
        "list_materials" => Ok(pretty(&json!({"materials": materials_json(&lock().doc),
            "defaults": "Generators with material_id null use the tree settings' bark_color (stems) or leaf_color (leaves)."}))),
        "add_material" => {
            let mut m = Material { name: arg_str(a, "name").unwrap_or("Material").to_string(), ..Default::default() };
            apply_material_patch(&mut m, a)?;
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let id = doc.add_material(m);
            if let Some(list) = a.get("assign_to").and_then(|v| v.as_array()) {
                for g in list {
                    let gid = g.as_u64().ok_or("assign_to must list generator ids")? as u32;
                    doc.get_mut(gid).ok_or_else(|| format!("no generator with id {gid}"))?.material = Some(id);
                }
            }
            sh.doc = doc;
            sh.commit();
            let v = material_json(&sh.doc, sh.doc.material(id).unwrap());
            Ok(pretty(&json!({"added": v})))
        }
        "update_material" => {
            let id = arg_u32(a, "id").ok_or("id is required")?;
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let m = doc.material_mut(id).ok_or_else(|| format!("no material with id {id}"))?;
            apply_material_patch(m, a)?;
            sh.doc = doc;
            sh.commit();
            let v = material_json(&sh.doc, sh.doc.material(id).unwrap());
            Ok(pretty(&json!({"updated": v})))
        }
        "remove_material" => {
            let id = arg_u32(a, "id").ok_or("id is required")?;
            let mut sh = lock();
            if !sh.doc.remove_material(id) {
                return Err(format!("no material with id {id}"));
            }
            sh.commit();
            Ok(text(format!("removed material {id}")))
        }
        "assign_material" => {
            let gid = arg_u32(a, "generator_id").ok_or("generator_id is required")?;
            let mut sh = lock();
            let mid = material_arg(&sh.doc, a.get("material_id").unwrap_or(&Value::Null))?;
            let g = sh.doc.get_mut(gid).ok_or_else(|| format!("no generator with id {gid}"))?;
            g.material = mid;
            sh.commit();
            let g = gen_json(&sh.doc, sh.doc.get(gid).unwrap());
            Ok(pretty(&json!({"updated": g})))
        }
        "remove_generator" => {
            let id = arg_u32(a, "id").ok_or("id is required")?;
            let mut sh = lock();
            if sh.doc.get(id).is_none() {
                return Err(format!("no generator with id {id}"));
            }
            let n = sh.doc.remove(id);
            sh.fix_selection();
            sh.commit();
            Ok(text(format!("removed {n} generator(s)")))
        }
        "update_settings" => {
            let mut sh = lock();
            let mut doc = sh.doc.clone();
            let mut patch = a.clone();
            if let Some(n) = patch.as_object_mut().and_then(|o| o.remove("name")) {
                doc.name = n.as_str().unwrap_or("Tree").to_string();
            }
            if let Some(w) = patch.as_object_mut().and_then(|o| o.remove("branch_welding")) {
                let w = w.as_f64().ok_or("branch_welding must be a number 0..1")?.clamp(0.0, 1.0) as f32;
                for g in doc.generators.iter_mut().filter(|g| g.kind == GenKind::Branches) {
                    g.params.weld = w;
                }
            }
            if patch.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                merge_into(&mut doc.settings, &patch, "settings")?;
            }
            doc.settings.clamp();
            sh.doc = doc;
            sh.commit();
            Ok(pretty(&json!({"name": sh.doc.name, "settings": sh.doc.settings})))
        }
        "randomize_seed" => {
            let mut sh = lock();
            let t = sh.started.elapsed().as_nanos() as u64;
            let seed = (crate::grow::Rng::new(&[t, sh.doc.settings.seed as u64]).next_u64() % 100_000) as u32;
            sh.doc.settings.seed = seed;
            sh.commit();
            Ok(text(format!("seed is now {seed}")))
        }
        "select_generator" => {
            let mut sh = lock();
            match a.get("id").and_then(|v| v.as_u64()) {
                Some(id) => {
                    if sh.doc.get(id as u32).is_none() {
                        return Err(format!("no generator with id {id}"));
                    }
                    sh.selected = Some(id as u32);
                }
                None => sh.selected = None,
            }
            Ok(text(format!("selected {:?}", sh.selected)))
        }
        "set_view" => {
            let mut sh = lock();
            let doc_growth = sh.doc.growth.clone();
            let v = &mut sh.view;
            if let Some(x) = arg_f32(a, "yaw") {
                v.camera.yaw = x;
            }
            if let Some(x) = arg_f32(a, "pitch") {
                v.camera.pitch = x;
            }
            if let Some(x) = arg_f32(a, "distance") {
                v.camera.distance = x;
            }
            if let Some(x) = arg_f32(a, "fov") {
                v.camera.fov = x;
            }
            if let Some(t) = a.get("target").and_then(|t| t.as_array()) {
                if t.len() == 3 {
                    v.camera.target = [0, 1, 2].map(|i| t[i].as_f64().unwrap_or(0.0) as f32);
                }
            }
            v.camera.clamp();
            if let Some(l) = arg_str(a, "lighting") {
                v.lighting = crate::render::Lighting::parse(l).ok_or("lighting must be unlit, morning, noon, evening or night")?;
            }
            if let Some(m) = arg_str(a, "mode") {
                v.mode = match m.to_ascii_lowercase().as_str() {
                    "shaded" => RenderMode::Shaded,
                    "wireframe" => RenderMode::Wireframe,
                    "skeleton" => RenderMode::Skeleton,
                    _ => return Err("mode must be shaded, wireframe or skeleton".into()),
                };
            }
            if let Some(b) = a.get("wind_playing").and_then(|v| v.as_bool()) {
                v.wind_playing = b;
            }
            if let Some(g) = growth_arg(a, &doc_growth)? {
                v.growth = g;
                v.growth_playing = false;
            }
            if let Some(b) = a.get("growth_playing").and_then(|v| v.as_bool()) {
                if b && !v.growth_playing && v.growth >= 0.999 {
                    v.growth = 0.0;
                }
                v.growth_playing = b;
            }
            if let Some(l) = arg_u32(a, "lod") {
                v.lod = l.min(4);
            }
            if let Some(b) = a.get("show_grid").and_then(|v| v.as_bool()) {
                v.show_grid = b;
            }
            if let Some(b) = a.get("show_leaves").and_then(|v| v.as_bool()) {
                v.show_leaves = b;
            }
            if let Some(b) = a.get("show_helpers").and_then(|v| v.as_bool()) {
                v.show_helpers = b;
            }
            if let Some(b) = a.get("show_collision").and_then(|v| v.as_bool()) {
                v.show_collision = b;
            }
            if let Some(b) = a.get("highlight_selection").and_then(|v| v.as_bool()) {
                v.highlight_selection = b;
            }
            if a.get("frame").and_then(|v| v.as_bool()).unwrap_or(false) {
                let lod = sh.view.lod;
                let m = crate::billboard::lod_mesh(&sh.doc, lod).mesh;
                sh.view.camera.frame(m.height, m.radius);
            }
            Ok(pretty(&json!(sh.view)))
        }
        "get_stats" => {
            let doc = lock().doc.clone();
            if a.get("all_lods").and_then(|v| v.as_bool()).unwrap_or(false) {
                let all: Map<String, Value> = (0..5).map(|l| (format!("lod{l}"), stats_for(&doc, l))).collect();
                Ok(pretty(&Value::Object(all)))
            } else {
                Ok(pretty(&stats_for(&doc, arg_u32(a, "lod").unwrap_or(0).min(4))))
            }
        }
        "render_preview" => {
            let (doc, view, sel) = {
                let sh = lock();
                (sh.doc.clone(), sh.view.clone(), sh.selected)
            };
            let lod = arg_u32(a, "lod").unwrap_or(view.lod).min(4);
            let lm = crate::billboard::lod_mesh(&doc, lod);
            let growth = growth_arg(a, &doc.growth)?.unwrap_or(1.0);
            let m = if growth < 1.0 {
                let timing = crate::growth::Timing::new(&doc, &lm.skeleton);
                crate::growth::Grower::new(&doc, &lm.skeleton, &lm.mesh, &timing).mesh_at(&lm.mesh, growth)
            } else {
                lm.mesh
            };
            // the mature tree's frame (mesh_at keeps its bounds), so growth pictures line up
            let (frame_h, frame_r) = (m.height, m.radius);
            let w = arg_u32(a, "width").unwrap_or(768).clamp(64, 2048);
            let h = arg_u32(a, "height").unwrap_or(768).clamp(64, 2048);
            // auto-fit the tree unless the caller wants the window's exact camera
            let mut cam = view.camera;
            if !a.get("use_window_camera").and_then(|v| v.as_bool()).unwrap_or(false) {
                // the mature tree's frame, so growth pictures line up
                cam.frame(frame_h, frame_r);
            }
            if let Some(x) = arg_f32(a, "yaw") {
                cam.yaw = x;
            }
            if let Some(x) = arg_f32(a, "pitch") {
                cam.pitch = x;
            }
            if let Some(x) = arg_f32(a, "distance") {
                cam.distance = x;
            }
            if let Some(t) = a.get("target").and_then(|t| t.as_array()).filter(|t| t.len() == 3) {
                cam.target = [0, 1, 2].map(|i| t[i].as_f64().unwrap_or(0.0) as f32);
            }
            cam.clamp();
            let _ = sel;
            let mut tex = crate::render::offscreen_textures(&doc);
            if let Some(atlas) = lm.atlas {
                tex.insert(crate::billboard::BILLBOARD_MAT, (atlas, true));
            }
            let lighting = match arg_str(a, "lighting") {
                Some(l) => crate::render::Lighting::parse(l).ok_or("lighting must be unlit, morning, noon, evening or night")?,
                None => view.lighting,
            };
            let wind_time = arg_f32(a, "wind_time").filter(|t| t.is_finite());
            let img = render_offscreen(&doc, &m, &cam, w, h, wind_time.unwrap_or(0.0), wind_time.is_some(), None, &tex, lighting);
            let png = png_bytes(&img)?;
            let mut note = format!("{} — {} triangles, height {:.2} m, LOD{}", doc.name, m.tris.len(), m.height, lod);
            if let Some(t) = wind_time {
                note.push_str(&format!(", wind at {t:.2} s"));
            }
            if growth < 1.0 {
                note.push_str(&format!(", growth {:.0}%", growth * 100.0));
            }
            if let Some(p) = arg_str(a, "save_to") {
                let p = expand(p);
                std::fs::write(&p, &png).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
                note.push_str(&format!("; saved to {}", p.display()));
            }
            Ok(image(png, note))
        }
        "capture_window" => {
            if s.headless {
                return Err("running without a window (--headless); use render_preview instead".into());
            }
            let (tx, rx) = mpsc::channel();
            lock().screenshot_waiters.push(tx);
            s.ctx.request_repaint();
            let png = match rx.recv_timeout(Duration::from_secs(8)) {
                Ok(r) => r?,
                Err(_) => return Err("the window did not draw in time (is it minimised or hidden?). render_preview works without the window.".into()),
            };
            let mut note = "ArborStudio window".to_string();
            if let Some(p) = arg_str(a, "save_to") {
                let p = expand(p);
                std::fs::write(&p, &png).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
                note.push_str(&format!("; saved to {}", p.display()));
            }
            Ok(image(png, note))
        }
        "export_tree" => {
            let path = expand(arg_str(a, "path").ok_or("path is required")?);
            if !path.is_absolute() {
                return Err("path must be absolute".into());
            }
            let fmt = match arg_str(a, "format") {
                Some(f) => Format::parse(f).ok_or("format must be obj, glb or json")?,
                None => Format::from_path(&path).ok_or("give format (obj, glb or json) or a path ending in .obj/.glb/.json")?,
            };
            let lods = a.get("include_lods").and_then(|v| v.as_bool()).unwrap_or(false);
            let bb = a.get("include_billboard").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut doc = lock().doc.clone();
            if let Some(g) = a.get("include_growth").and_then(|v| v.as_bool()) {
                doc.growth.export = g;
            }
            let r = export::export(&doc, &path, fmt, lods, bb)?;
            let files: Vec<String> = r.files.iter().map(|f| f.display().to_string()).collect();
            lock().log(Source::App, format!("exported {}", files.join(", ")));
            let mut out = json!({"files": files, "lods": r.lods});
            if let Some(g) = &r.growth {
                out["growth"] = g.clone();
            }
            if doc.budget.enabled {
                out["within_budget"] = json!(r.budget.iter().all(|f| !f.over));
                if let Some(w) = r.budget_warning(doc.budget.fit) {
                    out["budget_warning"] = json!(w);
                }
            }
            Ok(pretty(&out))
        }
        "export_variations" => {
            let dir = expand(arg_str(a, "folder").ok_or("folder is required")?);
            if !dir.is_absolute() {
                return Err("folder must be absolute".into());
            }
            let fmt = match arg_str(a, "format") {
                Some(f) => Format::parse(f).ok_or("format must be obj, glb or json")?,
                None => Format::Glb,
            };
            let count = arg_u32(a, "count").ok_or("count is required")?;
            let lods = a.get("include_lods").and_then(|v| v.as_bool()).unwrap_or(false);
            let bb = a.get("include_billboard").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut doc = lock().doc.clone();
            if let Some(g) = a.get("include_growth").and_then(|v| v.as_bool()) {
                doc.growth.export = g;
            }
            let first = arg_u32(a, "first_seed").unwrap_or(doc.settings.seed);
            let v = export::export_variations(&doc, &dir, count, first, fmt, lods, bb)?;
            lock().log(Source::App, format!("exported {} variations to {}", v.len(), dir.display()));
            let list: Vec<Value> = v
                .iter()
                .map(|(s, f, w)| {
                    let mut e = json!({"seed": s, "files": f});
                    if let Some(w) = w {
                        e["budget_warning"] = json!(w);
                    }
                    e
                })
                .collect();
            let mut out = json!({"variations": list});
            if doc.budget.enabled {
                out["within_budget"] = json!(v.iter().all(|(_, _, w)| w.is_none()));
            }
            Ok(pretty(&out))
        }
        "save_project" => {
            let mut sh = lock();
            let path = match arg_str(a, "path") {
                Some(p) => expand(p),
                None => sh.file.clone().ok_or("no current file; give a path")?,
            };
            sh.commit();
            save_project(&sh.doc, &path)?;
            crate::recent::add(&path);
            sh.file = Some(path.clone());
            sh.dirty = false;
            Ok(text(format!("saved {}", path.display())))
        }
        "open_project" => {
            let path = expand(arg_str(a, "path").ok_or("path is required")?);
            let doc = load_project(&path)?;
            crate::recent::add(&path);
            let mut sh = lock();
            sh.replace_doc(doc);
            sh.file = Some(path);
            sh.dirty = false;
            Ok(pretty(&tree_json(&sh)))
        }
        "undo" => {
            let mut sh = lock();
            Ok(text(if sh.undo() { "undone" } else { "nothing to undo" }))
        }
        "redo" => {
            let mut sh = lock();
            Ok(text(if sh.redo() { "redone" } else { "nothing to redo" }))
        }
        _ => Err(format!("unknown tool '{name}'")),
    }
}

// ---------------------------------------------------------------- stdio bridge

fn http_post(port: u16, body: &str) -> Result<(u16, String), String> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("bad http response")?;
    let code = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Ok((code, body.to_string()))
}

fn healthy(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)).is_ok()
}

/// `--mcp-stdio`: relay newline-delimited JSON-RPC between stdin/stdout and the running window.
pub fn run_stdio_bridge(port: u16) {
    if !healthy(port) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe)
                .args(["--port", &port.to_string()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        for _ in 0..100 {
            if healthy(port) {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match http_post(port, &line) {
            Ok((200, body)) => {
                let one_line: String = body.trim().replace('\n', " ");
                let _ = writeln!(stdout, "{one_line}");
                let _ = stdout.flush();
            }
            Ok((_, _)) => {}
            Err(e) => {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let Some(id) = v.get("id") {
                        let err = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("ArborStudio window is not reachable on port {port}: {e}")}});
                        let _ = writeln!(stdout, "{err}");
                        let _ = stdout.flush();
                    }
                }
            }
        }
    }
}

/// `--mcp-stdio --headless`: answer MCP over stdin/stdout in this process, without a window.
pub fn run_stdio_inprocess(server: Server) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(out) = handle_message(&server, &line) {
            let _ = writeln!(stdout, "{out}");
            let _ = stdout.flush();
        }
    }
}

// ---------------------------------------------------------------- materials

fn material_schema(adding: bool) -> Value {
    let mut props = json!({
        "name": {"type": "string"},
        "color": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 3, "maxItems": 3, "description": "Colour, or tint over the texture (white = unchanged)"},
        "texture": {"type": ["string", "null"], "description": "Absolute path to a PNG/JPEG colour texture"},
        "normal_map": {"type": ["string", "null"], "description": "Absolute path to a PNG/JPEG tangent-space normal map (shown in the viewport, exported for engines)"},
        "normal_strength": {"type": "number", "minimum": 0, "maximum": 4, "description": "How strongly the normal map bends the lighting (1 = as painted)"},
        "normal_flip_green": {"type": "boolean", "description": "true for DirectX-style normal maps (green = down); default OpenGL style"},
        "roughness": {"type": "number", "minimum": 0, "maximum": 1},
        "tiling": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "Bark texture repeats [around, along] a stem"},
        "alpha_cutout": {"type": "boolean", "description": "Transparent texture pixels become holes (leaf cards)"},
        "double_sided": {"type": "boolean"}
    });
    if adding {
        props["assign_to"] = json!({"type": "array", "items": {"type": "integer"}, "description": "Generator ids that should use the new material"});
    } else {
        props["id"] = json!({"type": "integer"});
    }
    props
}

fn material_json(doc: &Document, m: &Material) -> Value {
    let users: Vec<Value> = doc.material_users(m.id).iter().map(|g| json!({"id": g.id, "name": g.name})).collect();
    let mut v = serde_json::to_value(m).unwrap();
    v["used_by"] = json!(users);
    if let Some(t) = &m.texture {
        v["texture_status"] = json!(match crate::textures::check(t) {
            Ok((w, h)) => match crate::textures::preview_size(t) {
                Some((pw, ph)) if (pw, ph) != (w, h) => format!("ok, {w}x{h} (exports use the file at full size; the viewport and the billboard render from a {pw}x{ph} copy)"),
                _ => format!("ok, {w}x{h}"),
            },
            Err(e) => e,
        });
    }
    v
}

fn materials_json(doc: &Document) -> Value {
    json!(doc.materials.iter().map(|m| material_json(doc, m)).collect::<Vec<_>>())
}

/// Parse a material_id argument: null = default look, otherwise it must exist.
fn material_arg(doc: &Document, v: &Value) -> Result<Option<u32>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let id = v.as_u64().ok_or("material_id must be an integer or null")? as u32;
    if doc.material(id).is_none() {
        return Err(format!("no material with id {id}; see list_materials"));
    }
    Ok(Some(id))
}

/// Apply the material fields present in the tool arguments, checking image paths.
fn apply_material_patch(m: &mut Material, a: &Value) -> Result<(), String> {
    let mut patch = Map::new();
    for k in ["name", "color", "texture", "normal_map", "normal_strength", "normal_flip_green", "roughness", "tiling", "alpha_cutout", "double_sided"] {
        if let Some(v) = a.get(k) {
            let mut v = v.clone();
            if (k == "texture" || k == "normal_map") && v.is_string() {
                let p = expand(v.as_str().unwrap());
                if !p.is_absolute() {
                    return Err(format!("{k} must be an absolute path"));
                }
                let ps = p.display().to_string();
                crate::textures::check(&ps)?;
                v = json!(ps);
            }
            patch.insert(k.to_string(), v);
        }
    }
    merge_into(m, &Value::Object(patch), "material")?;
    m.clamp();
    Ok(())
}

// ---------------------------------------------------------------- forces

fn force_kind(a: &Value) -> Result<Option<ForceKind>, String> {
    match arg_str(a, "kind").map(|k| k.to_ascii_lowercase()) {
        None => Ok(None),
        Some(k) if k == "direction" || k == "directional" => Ok(Some(ForceKind::Direction)),
        Some(k) if k == "attractor" || k == "point" => Ok(Some(ForceKind::Attractor)),
        Some(k) => Err(format!("unknown force kind '{k}': use direction or attractor")),
    }
}

fn apply_force_patch(f: &mut Force, a: &Value) -> Result<(), String> {
    let mut patch = Map::new();
    for k in ["name", "direction", "position", "strength", "enabled"] {
        if let Some(v) = a.get(k) {
            patch.insert(k.into(), v.clone());
        }
    }
    merge_into(f, &Value::Object(patch), "force")?;
    f.clamp();
    Ok(())
}
