// <mnemo-graph graph="{nodes, edges}" [mode="wiki"]>: draws a graph with Cytoscape.
// Nodes are {label, url, stub, color} plus `distance` (a neighborhood) or `degree` (the whole
// wiki, mode="wiki"); edges are [index, index]. In a neighborhood, the node at distance 0 is the
// center, and buttons choose how many links away from it to show. Important nodes (near the
// center, or many links) get larger labels. Labels keep their size on screen at any zoom, and
// are placed most important first, leaving out any that would overlap one already placed;
// zooming in makes room for more, and pointing at a node shows its label.
// Everything lives in a
// shadow root: Datastar's live patches only change the `graph` attribute, and the drawing, its
// zoom and the chosen number of links survive them. New nodes are placed beside their
// neighbors; only choosing another number of links lays the graph out again.
const HOPS = [1, 2, 3];
// The chosen number of links per page, kept while navigating within this tab.
const chosen = new Map();

class MnemoGraph extends HTMLElement {
  static observedAttributes = ["graph"];

  get wiki() {
    return this.getAttribute("mode") === "wiki";
  }

  connectedCallback() {
    if (!this.shadowRoot) {
      const root = this.attachShadow({ mode: "open" });
      root.innerHTML = `<style>
          :host { display: flex; flex-direction: column; }
          .hops { display: flex; gap: 0.4rem; padding: 0.4rem; }
          [hidden] { display: none; }
          button { font: inherit; font-size: 0.85rem; padding: 0.15rem 0.6rem; cursor: pointer;
            border: 1px solid var(--line); border-radius: 999px; background: transparent; color: var(--muted); }
          button[aria-pressed="true"] { border-color: var(--accent); color: var(--fg); }
          .canvas { flex: 1; min-height: 0; position: relative; }
        </style>
        <div class="hops" role="group" aria-label="Links to follow">${HOPS.map(
          (n) => `<button type="button" data-hops="${n}">${n} ${n === 1 ? "link" : "links"}</button>`,
        ).join("")}</div>
        <div class="canvas"></div>`;
      if (this.wiki) root.querySelector(".hops").hidden = true;
      // Fit the drawing again when the canvas changes size (window resized, phone rotated).
      new ResizeObserver(() => {
        this.cy?.resize();
        this.cy?.fit(undefined, 20);
      }).observe(root.querySelector(".canvas"));
      root.querySelector(".hops").addEventListener("click", (event) => {
        const hops = Number(event.target.dataset?.hops);
        if (hops) {
          chosen.set(location.pathname, hops);
          this.draw(true);
        }
      });
    }
    // Cytoscape caches where its canvas is on screen and drops the cache on scroll events of
    // the canvas's ancestors, but it cannot see past the shadow root to the page. Without this,
    // clicks land off by the distance the page was scrolled.
    this.onScroll = () => {
      if (this.scrollPending) return;
      this.scrollPending = true;
      requestAnimationFrame(() => {
        this.scrollPending = false;
        this.cy?.resize();
      });
    };
    window.addEventListener("scroll", this.onScroll, { capture: true, passive: true });
    this.draw(true);
  }

  attributeChangedCallback() {
    if (this.isConnected && this.cy) this.draw(false);
  }

  disconnectedCallback() {
    window.removeEventListener("scroll", this.onScroll, { capture: true });
    this.cy?.destroy();
    this.cy = null;
  }

  // The elements to show, with nodes identified by their URL so updates can match them.
  elements() {
    let data;
    try {
      data = JSON.parse(this.getAttribute("graph") || "{}");
    } catch {
      return null;
    }
    const hops = chosen.get(location.pathname) ?? 2;
    for (const button of this.shadowRoot.querySelectorAll("button")) {
      button.setAttribute("aria-pressed", String(Number(button.dataset.hops) === hops));
    }
    const css = getComputedStyle(this);
    const all = data.nodes || [];
    const maxDegree = Math.max(1, ...all.map((n) => n.degree || 0));
    // 0 (least) to 1 (most important): the center and its neighbors, or the most-linked.
    const weight = (n) =>
      this.wiki ? Math.sqrt((n.degree || 0) / maxDegree) : [1, 0.6, 0.25, 0][n.distance] ?? 0;
    const nodes = all
      .filter((n) => this.wiki || n.distance <= hops)
      .map((n) => ({
        group: "nodes",
        data: {
          ...n,
          id: n.url,
          // Pointing at a page shows its spheres with its title.
          hoverLabel: n.spheres && n.spheres.length ? `${n.label} · ${n.spheres.join(", ")}` : n.label,
          fill: n.color == null ? cssColor(css, "--bg") : cssColor(css, `--type-${n.color}`),
          weight: weight(n),
          font: 11 + 5 * weight(n),
          labelSize: 11 + 5 * weight(n),
          labelGap: 4,
          size: this.wiki ? 10 + 18 * weight(n) : 16,
        },
      }));
    const shown = new Set(nodes.map((n) => n.data.id));
    const edges = (data.edges || [])
      .map(([a, b]) => [all[a]?.url, all[b]?.url])
      .filter(([a, b]) => shown.has(a) && shown.has(b))
      .map(([a, b]) => ({ group: "edges", data: { id: `${a} ${b}`, source: a, target: b } }));
    return { nodes, edges };
  }

  // Draws the graph; `relayout` arranges all nodes anew and fits them, otherwise the view and
  // the positions of nodes already shown stay as they are.
  draw(relayout) {
    const elements = this.elements();
    if (!elements) return;
    if (!this.cy) {
      this.cy = cytoscape({
        container: this.shadowRoot.querySelector(".canvas"),
        minZoom: 0.1,
        maxZoom: 3,
        style: style(getComputedStyle(this)),
      });
      this.cy.on("tap", "node", (event) => {
        window.location.href = event.target.data("url");
      });
      this.cy.on("zoom", () => this.scheduleLabels());
      this.cy.on("mouseover", "node", (event) => event.target.addClass("hover"));
      this.cy.on("mouseout", "node", (event) => event.target.removeClass("hover"));
    }
    const cy = this.cy;
    if (relayout) {
      cy.elements().remove();
      cy.add([...elements.nodes, ...elements.edges]);
      cy.layout(this.wiki ? WIKI_LAYOUT : LAYOUT).run();
      this.updateLabels();
      return;
    }
    const wanted = new Set([...elements.nodes, ...elements.edges].map((e) => e.data.id));
    cy.batch(() => {
      cy.elements().filter((e) => !wanted.has(e.id())).remove();
      const added = [];
      for (const node of elements.nodes) {
        const existing = cy.getElementById(node.data.id);
        if (existing.nonempty()) existing.data(node.data);
        else added.push(cy.add(node));
      }
      cy.add(elements.edges.filter((e) => cy.getElementById(e.data.id).empty()));
      for (const node of added) node.position(besideNeighbors(cy, node));
    });
    this.updateLabels();
  }

  scheduleLabels() {
    if (this.labelsPending) return;
    this.labelsPending = true;
    requestAnimationFrame(() => {
      this.labelsPending = false;
      this.updateLabels();
    });
  }

  // Labels the most important nodes first and leaves out any label that would overlap one
  // already shown. Cytoscape scales labels with the zoom, so their font size is divided by it
  // to keep them the same size on screen.
  updateLabels() {
    const cy = this.cy;
    if (!cy) return;
    const zoom = cy.zoom();
    const shown = [];
    const ranked = cy.nodes().sort((a, b) => b.data("weight") - a.data("weight"));
    cy.batch(() =>
      ranked.forEach((node) => {
        const box = labelBox(node);
        const free = !shown.some((other) => overlap(box, other));
        if (free) shown.push(box);
        node.toggleClass("quiet", !free);
        node.data({ labelSize: node.data("font") / zoom, labelGap: 4 / zoom });
      }),
    );
  }
}

// Rings by distance from the center.
const LAYOUT = {
  name: "concentric",
  animate: false,
  concentric: (node) => 10 - node.data("distance"),
  levelWidth: () => 1,
  minNodeSpacing: 30,
  spacingFactor: 0.9,
  avoidOverlap: true,
  nodeDimensionsIncludeLabels: true,
  padding: 20,
};

// Where a node's label is on screen, estimated from its length and font size, with a margin.
function labelBox(node) {
  const { x, y } = node.renderedPosition();
  const size = node.data("font");
  const width = node.data("label").length * size * 0.55 + 6;
  const top = y + node.renderedHeight() / 2 + 2;
  return { x1: x - width / 2, x2: x + width / 2, y1: top, y2: top + size * 1.3 };
}

function overlap(a, b) {
  return a.x1 < b.x2 && b.x1 < a.x2 && a.y1 < b.y2 && b.y1 < a.y2;
}

// Forces for the whole wiki: linked pages cluster, unlinked ones gather at the side.
const WIKI_LAYOUT = {
  name: "cose",
  animate: false,
  idealEdgeLength: () => 70,
  nodeRepulsion: () => 9000,
  componentSpacing: 60,
  padding: 20,
};

// A position for a new node: near the nodes it links to that already have one, else near the
// center (of a neighborhood) or the middle of the view.
function besideNeighbors(cy, node) {
  const placed = node.neighborhood("node").filter((n) => !n.same(node));
  const anchor = placed.nonempty() ? placed : cy.nodes("[distance = 0]");
  if (anchor.empty()) {
    const view = cy.extent();
    return { x: (view.x1 + view.x2) / 2, y: (view.y1 + view.y2) / 2 };
  }
  const x = anchor.map((n) => n.position("x"));
  const y = anchor.map((n) => n.position("y"));
  const mean = (values) => values.reduce((a, b) => a + b, 0) / Math.max(values.length, 1);
  const angle = Math.random() * 2 * Math.PI;
  return { x: mean(x) + 60 * Math.cos(angle), y: mean(y) + 60 * Math.sin(angle) };
}

function cssColor(css, name) {
  return css.getPropertyValue(name).trim();
}

function style(css) {
  const color = (name) => cssColor(css, name);
  return [
    {
      selector: "node",
      style: {
        label: "data(label)",
        "background-color": "data(fill)",
        "border-width": 1.5,
        "border-color": color("--muted"),
        color: color("--fg"),
        "font-size": "data(labelSize)",
        "text-valign": "bottom",
        "text-margin-y": "data(labelGap)",
        width: "data(size)",
        height: "data(size)",
      },
    },
    {
      selector: "node[?stub]",
      style: { "border-style": "dashed", "border-color": color("--stub"), color: color("--stub") },
    },
    {
      selector: "node[distance = 0]",
      style: { width: 24, height: 24, "border-width": 3, "border-color": color("--accent"), "font-weight": "bold" },
    },
    { selector: "node.quiet", style: { label: "" } },
    {
      selector: "node.hover",
      style: {
        label: "data(hoverLabel)",
        "z-index": 10,
        "text-background-color": color("--bg"),
        "text-background-opacity": 0.85,
        "text-background-padding": 2,
      },
    },
    { selector: "edge", style: { width: 1.2, "line-color": color("--muted"), "line-opacity": 0.45, "curve-style": "straight" } },
  ];
}

customElements.define("mnemo-graph", MnemoGraph);
