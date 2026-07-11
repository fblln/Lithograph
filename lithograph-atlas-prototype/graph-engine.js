// Layout, simulation, and canvas-rendering helpers for the Lithograph graph
// explorer. Pure functions / small classes; no DOM assumptions beyond a
// CanvasRenderingContext2D passed in explicitly.

import { nodeKindMeta, relKindMeta } from './graph-data.js';

export function clamp(v, lo, hi) {
  return Math.max(lo, Math.min(hi, v));
}

export function computeDegrees(nodes, relations) {
  const deg = {};
  for (const nd of nodes) deg[nd.id] = 0;
  for (const rel of relations) {
    deg[rel.source] = (deg[rel.source] || 0) + 1;
    deg[rel.target] = (deg[rel.target] || 0) + 1;
  }
  return deg;
}

export function nodeRadius(degree) {
  return 4.5 + clamp(degree, 0, 14) * 0.85;
}

export function buildAdjacency(relations) {
  const adj = {};
  for (const rel of relations) {
    (adj[rel.source] ||= { out: [], in: [] }).out.push(rel);
    (adj[rel.target] ||= { out: [], in: [] }).in.push(rel);
  }
  return adj;
}

// Breadth-first neighbor set within N hops of a root, following relations in
// either direction. Used for "focus depth".
export function neighborsWithin(rootId, relations, hops) {
  if (hops == null || hops === Infinity) return null; // null = "no restriction"
  const adj = {};
  for (const rel of relations) {
    (adj[rel.source] ||= []).push(rel.target);
    (adj[rel.target] ||= []).push(rel.source);
  }
  const seen = new Set([rootId]);
  let frontier = [rootId];
  for (let i = 0; i < hops; i++) {
    const next = [];
    for (const id of frontier) {
      for (const nb of adj[id] || []) {
        if (!seen.has(nb)) {
          seen.add(nb);
          next.push(nb);
        }
      }
    }
    frontier = next;
    if (!frontier.length) break;
  }
  return seen;
}

export function directNeighbors(nodeId, relations) {
  const set = new Set();
  for (const rel of relations) {
    if (rel.source === nodeId) set.add(rel.target);
    if (rel.target === nodeId) set.add(rel.source);
  }
  return set;
}

// ---------------------------------------------------------------------------
// Blast radius — directed reachability from a node along "impact" edges.
//   dependents   → who breaks if this changes (traverse incoming edges)
//   dependencies → what this relies on (traverse outgoing edges)
// Structural containment and lexical similarity are excluded: they aren't
// change-propagation paths.
// ---------------------------------------------------------------------------
export const IMPACT_EDGE_KINDS = new Set([
  'Calls', 'Imports', 'Implements', 'Inherits', 'TypeRefs', 'Usages', 'Ffi',
  'DataFlows', 'DependsOnPackage', 'ReadsEnv', 'RunsCommand', 'UsesImage',
  'BuildsImage', 'PublishesImage', 'Emits', 'ListensOn', 'References',
]);

export function blastRadius(rootId, relations, direction, allowKinds) {
  const adj = {};
  for (const rel of relations) {
    if (allowKinds && !allowKinds.has(rel.kind)) continue;
    if (direction === 'dependents') {
      (adj[rel.target] ||= []).push(rel.source);
    } else {
      (adj[rel.source] ||= []).push(rel.target);
    }
  }
  const depth = { [rootId]: 0 };
  let frontier = [rootId];
  let d = 0;
  while (frontier.length) {
    d++;
    const next = [];
    for (const id of frontier) {
      for (const nb of (adj[id] || [])) {
        if (depth[nb] === undefined) { depth[nb] = d; next.push(nb); }
      }
    }
    frontier = next;
  }
  return depth; // { id: hopDepth }, root at 0
}

// Warm (near) → cool (far) ramp for the blast overlay.
export function blastColor(depth, maxDepth) {
  if (depth === 0) return 'oklch(0.96 0.02 40)';
  const t = maxDepth <= 1 ? 1 : (depth - 1) / (maxDepth - 1);
  const L = (0.74 - 0.12 * t).toFixed(3);
  const C = (0.17 - 0.07 * t).toFixed(3);
  const H = (38 + 212 * t).toFixed(1);
  return `oklch(${L} ${C} ${H})`;
}

// ---------------------------------------------------------------------------
// Hierarchy forest (Contains / BelongsToModule / BelongsToPackage)
// ---------------------------------------------------------------------------
const HIER_MODE = {
  Contains: 'source-parent',
  BelongsToModule: 'target-parent',
  BelongsToPackage: 'target-parent',
};

export function buildForest(nodes, relations) {
  const parent = {};
  for (const rel of relations) {
    const mode = HIER_MODE[rel.kind];
    if (!mode) continue;
    const [p, c] = mode === 'source-parent' ? [rel.source, rel.target] : [rel.target, rel.source];
    if (parent[c]) continue;
    parent[c] = p;
  }
  const children = {};
  for (const [c, p] of Object.entries(parent)) {
    (children[p] ||= []).push(c);
  }
  const allIds = nodes.map((nd) => nd.id);
  const roots = allIds.filter((id) => !parent[id]);
  return { parent, children, roots };
}

// Tidy-ish forest layout: post-order width accumulation, then assign x/y.
export function layoutTree(nodes, relations, { spacingX = 46, spacingY = 100 } = {}) {
  const { children, roots } = buildForest(nodes, relations);
  const pos = {};
  let cursorX = 0;

  function widthOf(id) {
    const kids = children[id];
    if (!kids || !kids.length) return spacingX;
    return kids.reduce((sum, k) => sum + widthOf(k), 0);
  }

  function place(id, depth, xStart) {
    const kids = children[id] || [];
    if (!kids.length) {
      pos[id] = { x: xStart + spacingX / 2, y: depth * spacingY, vx: 0, vy: 0 };
      return xStart + spacingX;
    }
    let x = xStart;
    const childCenters = [];
    for (const k of kids) {
      const before = x;
      x = place(k, depth + 1, x);
      childCenters.push((before + x) / 2);
    }
    const center = (childCenters[0] + childCenters[childCenters.length - 1]) / 2;
    pos[id] = { x: center, y: depth * spacingY, vx: 0, vy: 0 };
    return x;
  }

  for (const rootId of roots) {
    cursorX = place(rootId, 0, cursorX) + spacingX * 0.6;
  }
  // Center around 0,0
  const xs = Object.values(pos).map((p) => p.x);
  const minX = Math.min(...xs), maxX = Math.max(...xs);
  const offset = (minX + maxX) / 2;
  for (const p of Object.values(pos)) p.x -= offset;
  return pos;
}

export function elbowPath(x1, y1, x2, y2) {
  const midY = (y1 + y2) / 2;
  return [
    [x1, y1],
    [x1, midY],
    [x2, midY],
    [x2, y2],
  ];
}

// ---------------------------------------------------------------------------
// Force simulation (shared by force + cluster layouts)
// ---------------------------------------------------------------------------
export function createForceSim(nodes, relations, opts = {}) {
  const { charge = 900, linkStrength = 0.06, linkDistance = 60, centerStrength = 0.01, damping = 0.86 } = opts;
  const pos = {};
  const n = nodes.length || 1;
  for (let i = 0; i < nodes.length; i++) {
    const angle = (i / n) * Math.PI * 2;
    const r = 80 + (i % 7) * 26;
    pos[nodes[i].id] = { x: Math.cos(angle) * r, y: Math.sin(angle) * r, vx: 0, vy: 0, fixed: false };
  }
  const anchors = {}; // id -> {x,y,strength}

  function setAnchor(id, x, y, strength = 0.02) {
    anchors[id] = { x, y, strength };
  }

  function tick() {
    const ids = nodes.map((nd) => nd.id);
    // Repulsion (O(n^2), fine for graphs of this size)
    for (let i = 0; i < ids.length; i++) {
      const a = pos[ids[i]];
      if (!a) continue;
      for (let j = i + 1; j < ids.length; j++) {
        const b = pos[ids[j]];
        if (!b) continue;
        let dx = a.x - b.x, dy = a.y - b.y;
        let distSq = dx * dx + dy * dy;
        if (distSq < 1) distSq = 1;
        const dist = Math.sqrt(distSq);
        const force = charge / distSq;
        const fx = (dx / dist) * force, fy = (dy / dist) * force;
        if (!a.fixed) { a.vx += fx; a.vy += fy; }
        if (!b.fixed) { b.vx -= fx; b.vy -= fy; }
      }
    }
    // Links
    for (const rel of relations) {
      const a = pos[rel.source], b = pos[rel.target];
      if (!a || !b) continue;
      const dx = b.x - a.x, dy = b.y - a.y;
      const dist = Math.max(1, Math.sqrt(dx * dx + dy * dy));
      const diff = (dist - linkDistance) * linkStrength;
      const fx = (dx / dist) * diff, fy = (dy / dist) * diff;
      if (!a.fixed) { a.vx += fx; a.vy += fy; }
      if (!b.fixed) { b.vx -= fx; b.vy -= fy; }
    }
    // Center / anchors
    for (const id of ids) {
      const p = pos[id];
      if (!p || p.fixed) continue;
      const anchor = anchors[id];
      if (anchor) {
        p.vx += (anchor.x - p.x) * anchor.strength;
        p.vy += (anchor.y - p.y) * anchor.strength;
      } else {
        p.vx += -p.x * centerStrength;
        p.vy += -p.y * centerStrength;
      }
    }
    // Integrate
    let maxSpeed = 0;
    for (const id of ids) {
      const p = pos[id];
      if (!p || p.fixed) continue;
      p.vx *= damping;
      p.vy *= damping;
      p.x += p.vx;
      p.y += p.vy;
      maxSpeed = Math.max(maxSpeed, Math.abs(p.vx), Math.abs(p.vy));
    }
    return maxSpeed;
  }

  return { pos, tick, setAnchor };
}

// Cluster nodes by nearest hierarchy root; leftover singleton roots are
// bucketed by kind so the cluster count stays readable.
export function clusterAssignment(nodes, relations) {
  const { parent, children, roots } = buildForest(nodes, relations);
  const rootHasChildren = new Set(roots.filter((r0) => children[r0] && children[r0].length));
  const clusterOf = {};
  function rootOf(id) {
    let cur = id;
    while (parent[cur]) cur = parent[cur];
    return cur;
  }
  for (const nd of nodes) {
    const root = rootOf(nd.id);
    if (rootHasChildren.has(root)) {
      clusterOf[nd.id] = 'tree:' + root;
    } else {
      clusterOf[nd.id] = 'kind:' + nd.kind;
    }
  }
  return clusterOf;
}

// Convex hull (Andrew's monotone chain). Returns hull points; for < 3 input
// points returns them as-is (a thick round-join stroke turns them into blobs).
export function convexHull(points) {
  const pts = [...points].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  if (pts.length < 3) return pts;
  const cross = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const lower = [];
  for (const p of pts) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) lower.pop();
    lower.push(p);
  }
  const upper = [];
  for (let i = pts.length - 1; i >= 0; i--) {
    const p = pts[i];
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) upper.pop();
    upper.push(p);
  }
  lower.pop();
  upper.pop();
  return lower.concat(upper);
}

export function hullPath(points) {
  if (!points.length) return '';
  let d = `M ${points[0][0]} ${points[0][1]}`;
  for (let i = 1; i < points.length; i++) d += ` L ${points[i][0]} ${points[i][1]}`;
  return d + ' Z';
}

// ---------------------------------------------------------------------------
// Screen <-> world transform helpers
// ---------------------------------------------------------------------------
export function screenToWorld(t, sx, sy) {
  return { x: (sx - t.x) / t.k, y: (sy - t.y) / t.k };
}
export function worldToScreen(t, wx, wy) {
  return { x: wx * t.k + t.x, y: wy * t.k + t.y };
}

export function hitTestNode(nodes, pos, degrees, transform, sx, sy) {
  const w = screenToWorld(transform, sx, sy);
  let best = null, bestDist = Infinity;
  for (const nd of nodes) {
    const p = pos[nd.id];
    if (!p) continue;
    const r = nodeRadius(degrees[nd.id] || 0) + 4;
    const dx = p.x - w.x, dy = p.y - w.y;
    const d = Math.sqrt(dx * dx + dy * dy);
    if (d <= r && d < bestDist) {
      best = nd;
      bestDist = d;
    }
  }
  return best;
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------
export function drawGraph(ctx, opts) {
  const {
    width, height, transform, nodes, relations, pos, degrees,
    visibleNodeKinds, visibleRelKinds, selectedId, hoveredId,
    neighborSet, edgeMode, bg, searchMatches,
  } = opts;

  ctx.save();
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, width, height);
  ctx.translate(transform.x, transform.y);
  ctx.scale(transform.k, transform.k);

  const activeFocus = selectedId || hoveredId;
  const dimAlpha = activeFocus ? 0.12 : 1;

  const visibleNodeSet = new Set();
  for (const nd of nodes) {
    if (visibleNodeKinds.has(nd.kind)) visibleNodeSet.add(nd.id);
  }

  // Edges
  ctx.lineWidth = Math.max(0.6, 1.1 / transform.k);
  for (const rel of relations) {
    if (!visibleRelKinds.has(rel.kind)) continue;
    if (!visibleNodeSet.has(rel.source) || !visibleNodeSet.has(rel.target)) continue;
    const a = pos[rel.source], b = pos[rel.target];
    if (!a || !b) continue;
    const meta = relKindMeta(rel.kind);
    const isNeighborEdge = activeFocus && (rel.source === activeFocus || rel.target === activeFocus);
    ctx.globalAlpha = activeFocus ? (isNeighborEdge ? 0.95 : dimAlpha) : 0.55;
    ctx.strokeStyle = meta ? meta.color : '#8886';
    ctx.beginPath();
    if (edgeMode === 'elbow') {
      const pts = elbowPath(a.x, a.y, b.x, b.y);
      ctx.moveTo(pts[0][0], pts[0][1]);
      for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]);
    } else {
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
    }
    ctx.stroke();
  }

  // Nodes
  for (const nd of nodes) {
    if (!visibleNodeSet.has(nd.id)) continue;
    const p = pos[nd.id];
    if (!p) continue;
    const meta = nodeKindMeta(nd.kind);
    const r = nodeRadius(degrees[nd.id] || 0) / transform.k ** 0.15;
    const isSelected = nd.id === selectedId;
    const isHovered = nd.id === hoveredId;
    const isNeighbor = neighborSet && neighborSet.has(nd.id);
    const isMatch = searchMatches && searchMatches.has(nd.id);
    let alpha = 1;
    if (activeFocus && !isSelected && !isHovered && !isNeighbor) alpha = dimAlpha;
    ctx.globalAlpha = alpha;
    ctx.beginPath();
    ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
    ctx.fillStyle = meta ? meta.color : '#999';
    ctx.fill();
    if (isSelected || isHovered) {
      ctx.lineWidth = 2 / transform.k;
      ctx.strokeStyle = '#fff';
      ctx.globalAlpha = 1;
      ctx.stroke();
    } else if (isMatch) {
      ctx.lineWidth = 1.6 / transform.k;
      ctx.strokeStyle = '#fff';
      ctx.globalAlpha = Math.max(alpha, 0.9);
      ctx.stroke();
    }
  }

  // Labels: selected/hovered/high-zoom/hub
  ctx.globalAlpha = 1;
  const showAllLabels = transform.k > 1.6;
  ctx.font = `${11 / transform.k}px ui-monospace, Menlo, monospace`;
  ctx.textBaseline = 'middle';
  for (const nd of nodes) {
    if (!visibleNodeSet.has(nd.id)) continue;
    const p = pos[nd.id];
    if (!p) continue;
    const isSelected = nd.id === selectedId;
    const isHovered = nd.id === hoveredId;
    const isNeighbor = neighborSet && neighborSet.has(nd.id);
    const deg = degrees[nd.id] || 0;
    const shouldLabel = isSelected || isHovered || (activeFocus && isNeighbor) || showAllLabels || deg >= 8;
    if (!shouldLabel) continue;
    if (activeFocus && !isSelected && !isHovered && !isNeighbor) continue;
    const r = nodeRadius(deg) / transform.k ** 0.15;
    ctx.fillStyle = isSelected || isHovered ? '#fff' : 'rgba(230,232,240,0.82)';
    ctx.fillText(nd.label, p.x + r + 5 / transform.k, p.y);
  }

  ctx.restore();
}
