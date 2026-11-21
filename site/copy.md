# ArborStudio landing page: copy

All page text, section by section. The markup in `index.html` follows this file.
Numbers marked (measured) were measured on 2026-11-20 with the current build on an Apple M2 Max
(12 cores), median of 20 seeds, through the MCP server over stdio. Numbers marked (from code) come from
the source or the docs in this repository.

## Meta

- Title: ArborStudio: game tree editor with an MCP server
- Description: Desktop editor for game-ready trees. GLB or OBJ with four LODs, a billboard and per-vertex
  wind weights. Every editor operation is an MCP tool, so your agent can grow the variants. Free, MIT.
- OG image text: "Game-ready trees. Ready for your favorite AI agent." / "GLB · OBJ · 4 LODs + billboard · wind weights · growth animation · 38 MCP tools"

## Hero

- Eyebrow: Tree editor for games · v0.3.0
- H1: Game-ready trees. Ready for your favorite AI agent.
- Subhead: ArborStudio is a desktop editor that grows trees from trunk, branch, root and leaf generators and
  exports them as GLB or OBJ with four LODs, a billboard and wind weights. Every operation in the editor is
  also an MCP tool, so Claude Code or any other MCP client can make the twelve variants you need while you
  do something else.
- For line: For game developers and environment artists who need many distinct trees and would rather
  script them than click out each one.
- Primary CTA: Download for macOS (14.8 MB) — the label follows the visitor's system: Windows (4.3 MB), Linux (6.3 MB). (from release v0.3.0)
- Next to CTA: Free and open source. MIT licence. Source on GitHub.
- Secondary (text link): Set up the MCP server →
- Screenshot: a hand-authored oak supplied by the person (home-folder path in the log and status bar shortened to the file name).
- Screenshot caption: An oak authored in ArborStudio: 744 stems, 15,390 leaves, 119,996 triangles at LOD0, grown in 95 ms. (measured, median of 20 runs)

## 1. An agent session, start to finish

- H2: Twelve birches for a northern forest, from one prompt.
- Lead: This is a session run against v0.1.0 while writing this page: the prompt, the calls the agent made,
  and what came out. Nothing is edited except long results, which are cut.
- Prompt: "Make 12 birch variants for a northern forest. Narrower crowns, a little early autumn colour.
  GLB with all LODs and billboards."
- Calls (excerpt, real timings):
  - load_preset {"name": "Birch"}
  - update_generator {"id": 2, "params": {"length": 0.36, "angle": 32}}
  - update_generator {"id": 3, "params": {"gravity": 0.7}}
  - update_settings {"autumn": 0.15, "leaf_color_variation": 0.3}
  - get_stats {"all_lods": true} → LOD0 35,520 triangles, 13.3 m tall (116 ms)
  - export_variations {"count": 12, "format": "glb", "include_lods": true, "include_billboard": true} → 24 files (1,434 ms)
- Result caption: Birch_v01.glb to Birch_v12.glb, seeds 1 to 12. Each file holds LOD0 to LOD3 and a
  billboard, plus a 4 × 2 billboard atlas PNG.
- Note: The agent here was a script driving the MCP server, not a chat client.
- CTA (middle): Download for macOS (14.8 MB) · Set up the MCP server →

## 2. What ends up in the file

- H2: Four LODs, a billboard, wind weights. One GLB.
- LOD table (Oak preset, seed 1, from code via get_stats): LOD0 32,014 · LOD1 16,982 · LOD2 10,234 ·
  LOD3 6,406 · Billboard 16 triangles.
- LOD note: Lower levels keep the skeleton and coarsen the mesh around it, so the silhouette holds when the
  engine switches.
- Wind: Two weights per vertex in TEXCOORD_1 (UV2): x is branch sway, 0 at the ground to 1 at the crown
  tips; y is leaf flutter. Your vertex shader applies them. ArborStudio does not ship an engine shader.
- Vertex colour: COLOR_0 carries tint, per-leaf variation, autumn colour and baked ambient occlusion.
- Materials: colour textures, normal maps, alpha-cutout leaf cards.
- Formats: GLB, OBJ with MTL, and JSON with the full branch skeleton for your own tools.
- Collision hulls, named for Unreal (UCX_) or Godot (-convcolonly) if you pick it.
- Engines: The docs describe the import into Unity, Unreal and Godot.

## 3. Growth animation (reworked in v0.3.0)

- Eyebrow: New in v0.3.0: the whole tree grows
- H2: The tree grows in your game, from a shoot to the crown you designed.
- Lead: The whole tree grows at once, the way trees do: every shoot grows a little each season, so a young tree is a small, slender tree with its own branches that thickens, widens and fills in with leaves. Branches come out of closed bark.
- Animation 1 (tall): the person's oak from the hero, 744 stems, growth 0 to 1, camera turning 50° and pulling back as the tree grows.
  Rendered with render_preview at 72 points of the growth. WebP, GIF fallback, still for reduced motion.
- Animation 2 (wide): closer to the crown, growth 0.2 to 1, camera pulling back with the tree's size.
- Facts: Per generator (speed, start size, start angle, length and radius curves, when parts start) ·
  Stages (up to eight, named) · GLB (morph targets on every LOD, a Grow clip and one clip per stage; Godot
  and Blender play them without a plugin) · Agents (set_growth, render_preview, include_growth).

## 4. Every part has its own controls

- H2: Real parameters, per generator.
- Lead: A tree is a stack of generators: trunk, branches, twigs, roots, leaves. Each one has its own set of
  settings, and most can follow a curve along the parent.
- Parameter names (from the editor): Count per parent, Start, End, Phyllotaxis, Length (× parent),
  Crown profile, Angle, Gravity, Wiggle, Kink, Split, Radius (× parent), Taper, Branch welding, Broken branches,
  Flutes, Bark lumps, Leaf shape, Leaf fold, Age, Autumn, Fallen leaves, Ambient occlusion.
- Timing table (measured, LOD0 / all five levels incl. billboard): Oak 32k tris 23 ms / 145 ms ·
  Birch 36k 24 ms / 110 ms · Willow 46k 40 ms / 208 ms · Pine 75k 94 ms / 397 ms.
- Note: Fast enough that the viewport follows a slider.

## 5. ArborStudio and SpeedTree

- H2: SpeedTree does more. Here is what ArborStudio does differently.
- Lead: SpeedTree is the reference for game vegetation and has far more features. ArborStudio is
  younger and smaller. It adds features when people ask for them in the issue tracker.
- Table:
  - Agent access: ArborStudio: MCP server, 38 tools | SpeedTree: no such interface
  - Price: ArborStudio: free | SpeedTree: paid licence
  - Source: ArborStudio: MIT, read and change the generator | SpeedTree: closed
  - Feature set: ArborStudio: smaller, grows from requests | SpeedTree: larger and more mature
- Link: Request a feature on GitHub →

## 6. Not for you if

- You need a hand-sculpted hero tree for a cinematic. You need SpeedTree's full
  feature set today. You work in a film or VFX pipeline.
- Rough edges (from code): The macOS build is not notarised yet, so macOS asks you once to allow it under
  System Settings › Privacy & Security. Variations ignore hand edits to single branches.
  Where many branches are welded close together, a growing tree can show small slivers in the bark for a moment.

## 7. Download

- H2: v0.3.0 for macOS, Windows and Linux.
- Files: macOS 11 or later, Apple silicon and Intel: ArborStudio-macos-universal.dmg, 14.8 MB ·
  Windows 10 or later, 64-bit: arbor_studio-windows-x86_64.msi, 4.3 MB ·
  Linux x86-64, X11 or Wayland: arbor_studio-linux-x86_64.tar.gz, 6.3 MB.
- Source: git clone https://github.com/sandboxly/arbor-studio && cargo build --release
- Docs teaser: 23 help pages with pictures, from a first tree to the full parameter reference.
  F1 in the app opens them. Written in Rust; the source is about one folder of modules you can read.
  → keep only the first two sentences plus "Written in Rust."

## Footer

- ArborStudio 0.3.0 · MIT licence · Source · Issues · Docs · Written in Rust.

## Open items (left off the page until they exist)

1. Screen recording of a real agent session, and the MCP clients tested.
2. Engines tested, with versions.
3. Regeneration loop (slider moving, tree updating).
4. Wind animation loop.
5. Who it is not for: confirm the list.
6. Pain quotes (5 to 10 from game developers; one as a pull quote).
7. Founder note (2 to 4 sentences, first person, signed).
8. Proof (stars, users, quotes): section left out until it exists.
