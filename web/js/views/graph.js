// Neighborhood graph: static two-column SVG layout (spec side left, code side right).
import { h, s, route, nodeHref, replaceChildren } from '../dom.js';
import { get, isAbort } from '../api.js';
import { titleCase, resolutionText, isWeakResolution, pct } from '../format.js';
import { loading, empty, errorBox, pageHeader } from '../components/state.js';
import { statusBadge, pill } from '../components/badge.js';

const W = 960;
const ROW = 46;
const NODE_W = 260;
const NODE_H = 32;
const LEFT_KINDS = new Set(['document', 'section', 'claim']);

function clip(text, n) {
  const t = String(text ?? '');
  return t.length > n ? t.slice(0, n - 1) + '…' : t;
}

function place(nodes, centerId) {
  const left = [];
  const right = [];
  let center = null;
  for (const n of nodes) {
    if (n.id === centerId) center = n;
    else if (LEFT_KINDS.has(n.kind)) left.push(n);
    else right.push(n);
  }
  const rows = Math.max(left.length, right.length, 1);
  const H = rows * ROW + 40;
  const pos = new Map();
  const col = (list, x) => {
    const top = (H - list.length * ROW) / 2 + (ROW - NODE_H) / 2;
    list.forEach((n, i) => pos.set(n.id, { x, y: top + i * ROW, side: x < W / 2 ? 'left' : 'right', node: n }));
  };
  col(left, 20);
  col(right, W - 20 - NODE_W);
  if (center) pos.set(center.id, { x: (W - 220) / 2, y: (H - 40) / 2, w: 220, h: 40, side: 'center', node: center });
  return { pos, H, left, right, center };
}

function anchor(p, toward) {
  const w = p.w || NODE_W;
  const hgt = p.h || NODE_H;
  const cy = p.y + hgt / 2;
  if (p.side === 'center') return { x: toward === 'left' ? p.x : p.x + w, y: cy };
  return { x: p.side === 'left' ? p.x + w : p.x, y: cy };
}

function drawEdge(e, pos) {
  const a = pos.get(e.from);
  const b = pos.get(e.to);
  if (!a || !b) return null;
  const weak = isWeakResolution(e.resolution);
  let d;
  let mid;
  if (a.side === b.side && a.side !== 'center') {
    // Same column: bulge outward so the line does not cross other nodes.
    const x = a.side === 'left' ? a.x : a.x + NODE_W;
    const ya = a.y + NODE_H / 2;
    const yb = b.y + NODE_H / 2;
    const bulge = x + (a.side === 'left' ? -1 : 1) * (14 + Math.min(40, Math.abs(ya - yb) / 4));
    d = `M ${x} ${ya} C ${bulge} ${ya}, ${bulge} ${yb}, ${x} ${yb}`;
    mid = { x: bulge, y: (ya + yb) / 2 };
  } else {
    const centerX = (p) => p.x + (p.w || NODE_W) / 2;
    const p1 = anchor(a, centerX(b) < centerX(a) ? 'left' : 'right');
    const p2 = anchor(b, centerX(a) < centerX(b) ? 'left' : 'right');
    d = `M ${p1.x} ${p1.y} L ${p2.x} ${p2.y}`;
    mid = { x: (p1.x + p2.x) / 2, y: (p1.y + p2.y) / 2 };
  }
  return s('g', { class: `gedge${weak ? ' gedge-weak' : ''}` },
    s('title', null, `${titleCase(e.kind)}: ${resolutionText(e.resolution)}, confidence ${pct(e.confidence)}`),
    s('path', { d, class: 'gedge-line', fill: 'none' }),
    s('text', { x: mid.x, y: mid.y - 3, class: 'gedge-label', 'text-anchor': 'middle' }, clip(titleCase(e.kind), 22)));
}

function drawNode(p, centerId) {
  const n = p.node;
  const w = p.w || NODE_W;
  const hgt = p.h || NODE_H;
  const isCenter = n.id === centerId;
  const body = [
    s('title', null, `${titleCase(n.kind)}: ${n.label}${n.detail ? ' - ' + n.detail : ''}`),
    s('rect', { x: p.x, y: p.y, width: w, height: hgt, rx: 6, class: 'gnode-box' }),
    s('text', { x: p.x + 10, y: p.y + hgt / 2 - 2, class: 'gnode-kind' }, n.kind),
    s('text', { x: p.x + 10, y: p.y + hgt / 2 + 11, class: 'gnode-label' }, clip(n.label, isCenter ? 28 : 34)),
  ];
  const cls = `gnode gnode-${n.kind}${n.status ? ' status-' + n.status : ''}${isCenter ? ' gnode-center' : ''}`;
  if (isCenter) return s('g', { class: cls, 'aria-label': `Center: ${n.label}` }, body);
  return s('a', { href: route('graph/' + n.id), class: cls, 'aria-label': `Recenter on ${titleCase(n.kind)} ${n.label}` }, body);
}

function neighborTable(nb, centerId) {
  const edgesFor = (id) => nb.edges.filter((e) => (e.from === id && e.to === centerId) || (e.to === id && e.from === centerId));
  const others = nb.nodes.filter((n) => n.id !== centerId);
  if (!others.length) return empty('This node has no neighbors.');
  return h('table', { class: 'dense' },
    h('caption', { class: 'sr-only' }, 'Neighbors of the selected node'),
    h('thead', null, h('tr', null, ['Node', 'Kind', 'Relationship', 'Actions'].map((c) => h('th', { scope: 'col' }, c)))),
    h('tbody', null, others.map((n) => {
      const es = edgesFor(n.id);
      const open = nodeHref(n.id);
      return h('tr', null,
        h('td', { class: 'wrap' }, n.label, n.status ? [' ', statusBadge(n.status)] : null, n.detail ? h('div', { class: 'muted small' }, n.detail) : null),
        h('td', null, titleCase(n.kind)),
        h('td', { class: 'small' }, es.length ? es.map((e) => h('div', null, `${e.from === centerId ? '→' : '←'} ${titleCase(e.kind)} (${resolutionText(e.resolution)})`)) : h('span', { class: 'muted' }, 'indirect')),
        h('td', null, h('a', { href: route('graph/' + n.id) }, 'Recenter'), open && !open.startsWith('#/graph/') ? [' · ', h('a', { href: open }, 'Open')] : null));
    })));
}

export function mount(root, ctx) {
  const id = ctx.params.id;
  const depth = ctx.query.depth === '2' ? 2 : 1;
  const body = h('div', null, loading('Loading graph'));
  const depthSel = h('select', { id: 'g-depth', onchange: (e) => { location.hash = route('graph/' + id, { depth: e.target.value }); } },
    [1, 2].map((d) => h('option', { value: d, selected: d === depth }, `Depth ${d}`)));
  root.append(h('div', { class: 'view' }, pageHeader('Graph', h('label', { for: 'g-depth', class: 'sr-only' }, 'Depth'), depthSel), body));

  function render(nb) {
    const { pos, H } = place(nb.nodes, nb.center);
    const edges = nb.edges.map((e) => drawEdge(e, pos));
    const nodes = [...pos.values()].map((p) => drawNode(p, nb.center));
    const svg = s('svg', { class: 'graph-svg', viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: 'group', 'aria-label': 'Relationship graph. A list of the same neighbors follows below.' }, edges, nodes);
    const center = nb.nodes.find((n) => n.id === nb.center);
    return h('div', { class: 'stack' },
      center ? h('p', null, h('strong', null, titleCase(center.kind) + ': '), center.label, center.status ? [' ', statusBadge(center.status)] : null,
        nodeHref(center.id) && !nodeHref(center.id).startsWith('#/graph/') ? [' ', h('a', { href: nodeHref(center.id) }, 'Open page')] : null) : null,
      nb.truncated ? h('p', { class: 'warn-line', role: 'note' }, 'Truncated: this neighborhood has more nodes than are shown.') : null,
      h('p', { class: 'muted small' }, 'Dashed lines are lexical, ambiguous or unresolved links. Specification nodes are on the left and code on the right.'),
      h('div', { class: 'graph-scroll', tabindex: '0', role: 'region', 'aria-label': 'Graph diagram (scrolls horizontally)' }, svg),
      h('section', { 'aria-labelledby': 'nb-h' }, h('h2', { id: 'nb-h' }, 'Neighbors'), neighborTable(nb, nb.center)),
      h('p', { class: 'row' }, pill(`${nb.nodes.length} nodes`, 'pill-muted'), pill(`${nb.edges.length} edges`, 'pill-muted')));
  }

  async function load(quiet) {
    if (!quiet) replaceChildren(body, loading('Loading graph'));
    try {
      replaceChildren(body, render(await get('/graph/neighborhood', { node: id, depth, limit: 60 }, ctx.signal)));
    } catch (e) {
      if (!isAbort(e)) replaceChildren(body, e.status === 404 ? empty('Node not found.', 'It may have been removed by a recent change.') : errorBox(e, () => load()));
    }
  }
  load();
  ctx.live(['scan.completed', 'relationship.*'], () => load(true), 3000);
  ctx.onResync(() => load(true));
  return { refresh: () => load(true) };
}

export function mountIndex(root) {
  const input = h('input', { id: 'g-node', type: 'text', placeholder: 'doc_…, clm_…, sym_… or src_…', autocomplete: 'off', class: 'mono' });
  root.append(h('div', { class: 'view' }, pageHeader('Graph'),
    h('p', null, 'The graph shows how a specification node and code nodes relate. Open it from a finding, claim, document or symbol, or enter a node id.'),
    h('form', { class: 'filters', onsubmit: (e) => { e.preventDefault(); const v = input.value.trim(); if (v) location.hash = route('graph/' + v); } },
      h('div', { class: 'field field-wide' }, h('label', { for: 'g-node' }, 'Node id'), input),
      h('button', { type: 'submit', class: 'btn btn-primary' }, 'Show neighborhood')),
    h('p', null, h('a', { href: route('specs') }, 'Browse specs'), ' · ', h('a', { href: route('code') }, 'Browse code'))));
}
