# ArborStudio

A procedural tree modeller with a built-in MCP server, so a coding agent can build, preview and export trees.

## Download

Ready-to-run builds are attached to each release on the GitHub **Releases** page: a disk image for macOS
(Apple silicon and Intel; drag ArborStudio to Applications), an installer for Windows (`.msi`) and a `.tar.gz`
for Linux. macOS may ask you to allow the app under System Settings > Privacy & Security the first time,
because the build is not notarised yet.

## Build from source

Install Rust (https://rustup.rs), then in this folder:

    cargo build --release                   # the program is target/release/arbor_studio

Linux also needs the usual window-system libraries, e.g. on Ubuntu:
`sudo apt install libgtk-3-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev`.

## Run

    cargo run --release                     # opens the window; MCP at http://127.0.0.1:7878/mcp
    arbor_studio --port 9000                # other MCP port (or env ARBOR_MCP_PORT)
    arbor_studio --mcp-stdio                # stdio MCP server; relays to the window and starts it if needed
    arbor_studio --headless --mcp-stdio     # stdio MCP server with no window at all (build scripts, CI)
    arbor_studio --headless                 # HTTP MCP server with no window
    arbor_studio --screenshot out.png       # save a picture of the window and exit
    arbor_studio --shot-script steps.json   # take the help's screenshots (see src/docshots.rs) and exit

## Website and help

The project website is https://sandboxly.github.io/arbor-studio/. `.github/workflows/docs.yml` builds it on
every push to `main` that changes `site/` or `docs/`: the landing page in `site/` becomes the root and the
user guide in `docs/` (plain HTML pages with screenshots) is published under `/docs/`. In the app, F1 or
**Help → ArborStudio Help** opens the guide in the default web browser (`src/help.rs`; set `ARBOR_HELP_URL`
to point the app at another copy, for example `file:///…/docs/`).

Both follow the Arbor Studio design system (`docs/ds/tokens.css` and `docs/ds/components.css`; dark
grounds, one forest-green accent, hairline borders) and use its typefaces Space Grotesk, Geist and
JetBrains Mono, bundled in `docs/fonts` under the SIL Open Font License. To preview the website locally,
copy `site/` into an empty folder and `docs/` into its `docs` subfolder, then open `index.html`.

## Menus and recent projects

On macOS the menus (File, Edit, Generators, Presets, View, Agent, Window, Help) are in the system menu bar with
the usual shortcuts (⌘N, ⌘O, ⌘S, ⇧⌘S, ⌘E, ⌘Z, ⇧⌘Z, ⌘Q); on Windows and Linux they are at the top of the
window. **File → Open recent** lists the last ten projects opened or saved (also those an agent opened or
saved), marks ones that were moved or deleted, and can be cleared. The list is kept in the settings folder
(macOS `~/Library/Application Support/Arbor Studio`, Windows `%APPDATA%\Arbor Studio`, Linux
`~/.config/arbor-studio`).

## Connect an agent

* Claude Code: `claude mcp add --transport http arbor-studio http://127.0.0.1:7878/mcp`
* stdio clients: `{"mcpServers": {"arbor-studio": {"command": "/path/to/arbor_studio", "args": ["--mcp-stdio"]}}}`

The window's **Agent → Connect a coding agent…** shows these with copy buttons.

## Tools

get_tree, get_parameter_reference, list_presets, load_preset, new_tree, add_generator, update_generator,
remove_generator, set_curve, list_nodes, edit_node, clear_node_edits, add_force, update_force, remove_force, set_pruning,
set_collision, set_triangle_budget, check_triangle_budget, set_growth, list_materials, add_material, update_material, remove_material, assign_material, update_settings, randomize_seed, select_generator, set_view, get_stats, render_preview,
capture_window, export_tree, export_variations, save_project, open_project, undo, redo.

## Releases (maintainers)

The repository builds itself on GitHub: every change on `main` is compiled on Linux, macOS and Windows
(`.github/workflows/ci.yml`), and a version tag such as `v0.1.0` builds the downloads and creates the
release with them (`.github/workflows/release.yml`).

The downloads are a disk image for macOS (`ArborStudio-macos-universal.dmg`), an installer for Windows
(`arbor_studio-windows-x86_64.msi`, made with WiX Toolset 5 from `windows/ArborStudio.wxs`) and a
`.tar.gz` for Linux. Running the Release workflow by hand with an existing tag rebuilds that version's
downloads.

macOS releases hold one `ArborStudio.app` for Apple silicon and Intel (`macos/bundle.sh` makes it,
`macos/dmg.sh` puts it on the disk image; both work locally:
`sh macos/bundle.sh target/release/arbor_studio <empty folder>`). They build with no setup and carry an ad-hoc signature, so Mac users confirm the first start once under System Settings >
Privacy & Security. To have them signed and notarised by Apple (opens with no warning), a paid Apple
Developer Program membership is needed. With the "Developer ID Application" certificate on your Mac, one
command sets everything up once the repository is on GitHub (it asks for your Apple Account email and an
app-specific password, checks them with Apple, and uploads only that certificate, encrypted):

    sh macos/setup-github-secrets.sh

It fills these repository secrets (Settings > Secrets and variables > Actions), which can also be set by
hand; the release does the rest:

* `MACOS_CERTIFICATE`: a "Developer ID Application" certificate (create it in Xcode > Settings > Accounts >
  Manage Certificates, or at developer.apple.com), exported from Keychain Access as .p12, then base64
  (`base64 -i cert.p12 | pbcopy`).
* `MACOS_CERTIFICATE_PASSWORD`: the password chosen when exporting the .p12.
* `APPLE_ID`: the Apple Account email of the membership.
* `APPLE_APP_PASSWORD`: an app-specific password for that account (account.apple.com > Sign-In and Security).
* `APPLE_TEAM_ID`: the 10-character team id (developer.apple.com > Account > Membership details).

Optional: `MACOS_BUNDLE_ID` (default `io.github.arbor-studio`) and an icon at `macos/AppIcon.icns`.

## Engine-neutral export

* **GLB (glTF 2.0)** and **OBJ + MTL**: Unity, Unreal, Godot, Blender. Metres, Y up.
* LOD meshes are named `<Tree>_LOD0..3` (the naming Unity and Unreal use for LOD groups). Every LOD is the
  same tree with less detail, so switching levels in an engine doesn't change the silhouette: each branch keeps
  its path (only the rings that shape it least are removed), thinner cross-sections are widened to keep their
  apparent thickness, and the leaves are an even subset of the full set, enlarged around their own centres
  so the crown keeps its outline and leaf area.
* GLB wind data: `TEXCOORD_1` (UV2) x = branch sway weight (0 at ground → 1 at tips), y = leaf flutter weight.
  `COLOR_0` holds the albedo colour times baked ambient occlusion (`ao_strength`).
* **Collision**: capsules fitted to the trunk and thick branches, exported as convex hulls. Naming `neutral`
  (`<Tree>_Collision_NN`), `unreal` (`UCX_<mesh>_NN`) or `godot` (`-convcolonly`); capsule data in GLB node
  extras and in the JSON file.
* **Growth animation** (Properties › Growth): the tree grows from a sprout to the mature tree on its own mesh,
  the whole tree at once, as trees grow (a young tree is a small, slender tree with its branches; every stem keeps
  its shape and grows in size, branches come out of closed bark, leaves keep coming so the crown gets denser), with per-generator growth settings, through named stages (Sprout, Sapling, Young, Mature by default). GLB gets one morph target per keyframe, a
  one-bone skin (so engines that keep morph targets only on skinned meshes import them) and animations driving
  the morph weights: `Grow` plus one clip per stage. OBJ gets one file per stage; JSON the growth timing of every
  stem and leaf. Tested by importing in Godot 4 and Blender.
* **Variations**: `export_variations` (File menu too) writes several seeds of the species as `<Tree>_v01..`.
* **JSON**: the full skeleton (stem polylines with radii, leaf positions/directions/normals/sizes) plus the
  generator settings, for custom engines.

## Materials

Each tree has a material library. A material has a colour (or tint over a texture), an optional PNG/JPEG colour
texture and normal map, roughness, tiling for bark, alpha cut-out for leaf cards and a two-sided flag.
Assign one material to any number of generators; generators without one use the default bark/leaf colours.
GLB exports embed the textures; OBJ exports copy them next to the .obj and reference them from the .mtl.
For leaf textures with transparency, use leaf shape "card" and turn on cut-out.

Bark textures wrap once around each stem per "around" tiling, and repeat along it once per local
circumference (× "along" tiling), so the image keeps its proportions on thick trunks and thin roots alike
(measured on the real bark surface, buttress flare included). At joints the collar between the opening and
the branch carries the branch's own texture, continued from the branch, so its bark flows into the
joint instead of being smeared between the parent's and the branch's unrelated coordinates.
Normal maps light the bark in the viewport (per pixel); `normal_strength` scales them and
`normal_flip_green` is for DirectX-style maps (glTF and the default expect OpenGL style, green = up).

The viewport has lighting switches (Unlit, Morning, Noon, Evening, Night) in its top-left corner; agents
use `set_view` `lighting`, and `render_preview` follows the window's choice or its own `lighting`.

## Roots, forces, pruning and billboards

* **Roots** (generator kind `roots`, on a trunk): roots spread from the trunk base, sunk in by `buried`, and
  never rise out of the soil. Their `gravity` (0..1) is how steeply they head down: 0 runs along the ground
  surface, 1 dives straight down; they turn towards that pitch gradually, so they leave the trunk near the
  surface. Add branches under roots for side roots (the same gravity rule). The trunk's buttress flare forms ridges towards the roots.
* **Forces**: `direction` pushes all growth one way; `attractor` pulls towards (or, negative, pushes from) a point.
  Each generator's `force_response` scales the effect.
* **Pruning**: trims branches and leaves to a box or ellipsoid (trunk only cut at the top, roots never).
* **Billboard LOD (LOD 4)**: the tree pictured from 8 sides into an atlas, on four crossed two-faced cards.
  Exports add it as `_LOD4` (or `_Billboard`) and save `<name>_billboard.png` next to the file.

## Geometry control

* **Curves**: most settings can be shaped by a curve (value × curve). Along the parent: length, angle,
  thickness, density, angle/length variation, kink, broken branches, leaf size/fold/bend/face-up. Along the
  stem itself: thickness profile, taper strength, gravity, wiggle, jitter, force response, flute depth,
  flatten, bark lumps. Each curve can instead run up the whole tree's height (0 = ground, 1 = top), the
  same for every branch level. Taper example: `set_curve` curve `taper`, preset `ramp_up` = the trunk keeps
  its thickness low down and narrows towards the top (the tip still ends at the Taper setting). In the
  app: the small curve icon at the end of each setting's row opens a popup curve editor (switch the curve
  on, pick the x axis, drag points, double-click to add, right-click to remove); the icon turns green when
  a curve is on.
* **Spine**: kink, jitter, spacing in metres (`interval`) instead of a count, and splitting (`splits`, `split_at`,
  `split_angle`): each stem of a trunk, branch or root generator divides into 2-5 leaders that share its wood,
  taper to thin tips and carry the children above the split.
* **Skin**: flutes, flatten, bark lumps, weld (collar where a branch meets its parent), broken branch stubs.
* **Leaves**: fold, curl, face the sky, or your own OBJ model (`leaf_shape: mesh`, `leaf_mesh`).
* **Node editing**: pick "Branches" in the toolbar and click one branch to delete it or change its length,
  thickness, angle and rotation (`list_nodes` / `edit_node`).
* **Tree**: age (growth stage), fallen leaves (season), autumn colours, ambient occlusion.
* **Branch joints** (`join_branches`, `join_method`): `stitch` cuts an opening in the parent that fits the
  whole branch base and stitches it to the branch (one watertight mesh); `weld` leaves the parent whole and
  wraps the branch base onto its surface with a curved fillet whose normals, colour and wind blend into the
  parent (the welding approach production tree tools describe; fewer triangles). Weld amount widens either.

## License

MIT, see `LICENSE`.
