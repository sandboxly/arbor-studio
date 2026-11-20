// ArborStudio Help: builds the side navigation, the topic filter and the previous/next links.
// Add new pages here; each entry is [file, title, extra search words].
const HELP_PAGES = [
  ["Getting started", [
    ["index.html", "Welcome", "overview start quick"],
    ["installation.html", "Installing and starting", "download install macos windows linux command line build"],
    ["first-tree.html", "Your first tree", "tutorial walkthrough beginner"],
    ["interface.html", "The window", "toolbar panels viewport menu status camera navigation"],
  ]],
  ["Building trees", [
    ["concepts.html", "How trees are built", "generator hierarchy seed relative count parent"],
    ["generators.html", "Generators and placement", "count start end phyllotaxis whorl randomness spacing interval add remove"],
    ["spine.html", "Stem shape", "length height profile crown angle lean gravity wiggle segments kink jitter force response"],
    ["skin.html", "Thickness and bark surface", "radius taper flare buttress sides flutes flatten lumps bumps broken stubs"],
    ["roots.html", "Roots", "roots ground buried gravity side roots flutes"],
    ["leaves.html", "Leaves", "leaf shape size fold curl face sky card mesh obj frond needle"],
    ["curves.html", "Curves", "curve along tree height popup editor shape"],
    ["joints.html", "Branch joints", "join weld stitch collar opening watertight"],
  ]],
  ["Appearance", [
    ["materials.html", "Materials and textures", "material texture normal map tiling roughness cutout two-sided bark leaf"],
    ["tree-settings.html", "Tree settings", "seed scale age colour autumn season fallen ambient occlusion wind"],
    ["forces.html", "Forces, pruning and collision", "force direction attractor pruning box ellipsoid collision capsule physics"],
    ["node-editing.html", "Editing single branches", "node branch delete length thickness rotate pick"],
  ]],
  ["Output", [
    ["lod.html", "Levels of detail", "lod billboard atlas reduction silhouette triangle budget polygon count warning"],
    ["growth.html", "Growth animation", "growth grow animation sprout sapling stage speed radius sync start offset morph target blend shape shape key regrow clip"],
    ["export.html", "Exporting", "export glb gltf obj json unity unreal godot variations engine"],
    ["projects.html", "Projects and files", "save open recent undo redo project arbor.json"],
    ["agents.html", "Coding agents (MCP)", "mcp agent claude tools automation headless"],
  ]],
  ["Reference", [
    ["reference.html", "Parameter reference", "all parameters ranges table"],
    ["shortcuts.html", "Keyboard and mouse", "shortcuts keys mouse"],
  ]],
];

(function () {
  const nav = document.getElementById("nav");
  if (!nav) return;
  const here = location.pathname.split("/").pop() || "index.html";
  const flat = [];
  let html = '<a class="brand" href="index.html"><span class="brand-name">ArborStudio</span><span class="brand-label">Help</span></a>';
  // the help is published under docs/ of the project website; offline copies link to the online home
  const home = location.protocol.startsWith("http") ? "../" : "https://sandboxly.github.io/arbor-studio/";
  html += `<a class="home" href="${home}">← ArborStudio home</a>`;
  html += '<input type="search" id="nav-filter" placeholder="Filter topics" aria-label="Filter topics">';
  let section = "";
  for (const [group, pages] of HELP_PAGES) {
    html += `<h3>${group}</h3>`;
    for (const [file, title, words] of pages) {
      flat.push([file, title]);
      if (file === here) section = group;
      const cur = file === here ? " current" : "";
      html += `<a class="item${cur}" href="${file}" data-words="${(title + " " + words).toLowerCase()}">${title}</a>`;
    }
  }
  nav.innerHTML = html;

  const filter = document.getElementById("nav-filter");
  filter.addEventListener("input", () => {
    const q = filter.value.trim().toLowerCase();
    for (const a of nav.querySelectorAll("a.item")) {
      a.classList.toggle("hidden", q !== "" && !a.dataset.words.includes(q));
    }
    for (const h of nav.querySelectorAll("h3")) {
      let el = h.nextElementSibling, any = false;
      while (el && el.tagName === "A") { if (!el.classList.contains("hidden")) any = true; el = el.nextElementSibling; }
      h.style.display = any ? "" : "none";
    }
  });

  const i = flat.findIndex(([f]) => f === here);
  const main = document.querySelector("main");
  // section eyebrow above the page title
  const h1 = main && main.querySelector("h1");
  if (h1 && section) h1.insertAdjacentHTML("beforebegin", `<p class="eyebrow">${section}</p>`);
  if (i >= 0 && main) {
    const prev = flat[i - 1], next = flat[i + 1];
    let p = '<div class="pager">';
    if (prev) p += `<a class="prev ab-btn ab-btn--ghost ab-btn--sm" href="${prev[0]}">← ${prev[1]}</a>`;
    if (next) p += `<a class="next ab-btn ab-btn--ghost ab-btn--sm" href="${next[0]}">${next[1]} →</a>`;
    p += "</div>";
    main.insertAdjacentHTML("beforeend", p);
  }
})();
