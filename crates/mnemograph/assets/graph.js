// <mnemo-graph graph="{nodes, edges}">: draws a neighborhood graph with Cytoscape.
// Nodes are {label, url, stub, distance, color}; edges are [index, index]. The node at distance
// 0 is the center; buttons choose how many links away from it to show. Everything lives in a
// shadow root: Datastar's live patches only change the `graph` attribute, and the drawing, its
// zoom and the chosen number of links survive them. New nodes are placed beside their
// neighbors; only choosing another number of links lays the graph out again.
const HOPS = [1, 2, 3];
// The chosen number of links per page, kept while navigating within this tab.
const chosen = new Map();

class MnemoGraph extends HTMLElement {
  static observedAttributes = ["graph"];

  connectedCallback() {
    if (!this.shadowRoot) {
      const root = this.attachShadow({ mode: "open" });
      root.innerHTML = `<style>
          :host { display: flex; flex-direction: column; }
          .hops { display: flex; gap: 0.4rem; padding: 0.4rem; }
          button { font: inherit; font-size: 0.85rem; padding: 0.15rem 0.6rem; cursor: pointer;
            border: 1px solid var(--line); border-radius: 999px; background: transparent; color: var(--muted); }
          button[aria-pressed="true"] { border-color: var(--accent); color: var(--fg); }
          .canvas { flex: 1; min-height: 0; position: relative; }
        </style>
        <div class="hops" role="group" aria-label="Links to follow">${HOPS.map(
          (n) => `<button type="button" data-hops="${n}">${n} ${n === 1 ? "link" : "links"}</button>`,
        ).join("")}</div>
        <div class="canvas"></div>`;
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
    const nodes = all
      .filter((n) => n.distance <= hops)
      .map((n) => ({
        group: "nodes",
        data: {
          ...n,
          id: n.url,
          fill: n.color == null ? cssColor(css, "--bg") : cssColor(css, `--type-${n.color}`),
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
        minZoom: 0.3,
        maxZoom: 3,
        style: style(getComputedStyle(this)),
      });
      this.cy.on("tap", "node", (event) => {
        window.location.href = event.target.data("url");
      });
    }
    const cy = this.cy;
    if (relayout) {
      cy.elements().remove();
      cy.add([...elements.nodes, ...elements.edges]);
      cy.layout(LAYOUT).run();
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

// A position for a new node: near the nodes it links to that already have one.
function besideNeighbors(cy, node) {
  const placed = node.neighborhood("node").filter((n) => !n.same(node));
  const anchor = placed.nonempty() ? placed : cy.nodes("[distance = 0]");
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
        "font-size": 16,
        "min-zoomed-font-size": 6,
        "text-valign": "bottom",
        "text-margin-y": 4,
        width: 16,
        height: 16,
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
    { selector: "edge", style: { width: 1.2, "line-color": color("--muted"), "line-opacity": 0.45, "curve-style": "straight" } },
  ];
}

customElements.define("mnemo-graph", MnemoGraph);
