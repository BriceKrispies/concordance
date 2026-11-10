import { h, route, replaceChildren } from '../dom.js';
import { get, post, isAbort } from '../api.js';
import { getState, subscribe, loadStatus } from '../store.js';
import { when, ago, duration, titleCase, rangeStr, plural, STATUS_HELP } from '../format.js';
import { statusBadge, severityBadge } from '../components/badge.js';
import { loading, empty, errorBox, inlineError, pageHeader } from '../components/state.js';

const TILES = [
  ['claims', 'Claims', route('specs')],
  ['obligation_claims', 'Obligation claims', route('specs')],
  ['verified', 'Verified', route('findings', { status: 'verified' })],
  ['violated', 'Violated', route('findings', { status: 'violated' })],
  ['unverified', 'Unverified', route('findings', { status: 'unverified' })],
  ['stale', 'Stale', route('findings', { status: 'stale' })],
  ['ambiguous', 'Ambiguous', route('findings', { status: 'ambiguous' })],
  ['undocumented', 'Undocumented', route('findings', { status: 'undocumented' })],
];

function rootsList(title, list) {
  return h('div', null, h('dt', null, title),
    h('dd', null, list && list.length ? h('ul', { class: 'plain mono' }, list.map((r) => h('li', null, r))) : h('span', { class: 'muted' }, 'none')));
}

function tiles(counts) {
  return h('ul', { class: 'tiles' }, TILES.map(([key, label, href]) => {
    const n = counts ? counts[key] : 0;
    const cls = STATUS_HELP[key] ? `tile tile-${key}` : 'tile';
    return h('li', null, h('a', { class: cls, href }, h('span', { class: 'tile-n' }, n), h('span', { class: 'tile-l' }, label)));
  }));
}

function scanPanel(st) {
  const scan = st.scan;
  const last = st.last_scan;
  return h('section', { class: 'panel', 'aria-labelledby': 'scan-h' },
    h('h2', { id: 'scan-h' }, 'Scan'),
    scan
      ? h('p', null, h('strong', null, 'Scanning'), ` (${scan.reason || 'scan'}): ${titleCase(scan.phase)}`,
        scan.total ? ` ${scan.done}/${scan.total}` : '', ' ',
        h('progress', { max: scan.total || 1, value: scan.total ? scan.done : 0, 'aria-label': 'Scan progress' }))
      : h('p', { class: 'muted' }, 'Idle.', st.watching ? ' Watching for file changes.' : ' Not watching for changes.'),
    last
      ? h('p', null, `Last completed scan: revision ${last.revision}, `, h('time', { title: when(last.at) }, ago(last.at)),
        `, took ${duration(last.duration_ms)}, ${plural(last.changed_files, 'file')} changed.`)
      : h('p', { class: 'muted' }, 'No scan has completed yet.'));
}

function semanticPanel(st, onAnalyze, feedback) {
  const sem = st.semantic || {};
  const body = [];
  if (!sem.enabled) {
    body.push(h('p', { class: 'note' }, 'Structural mode: behavioral claims cannot be verified without semantic analysis.'));
    body.push(h('button', { type: 'button', class: 'btn', disabled: true, title: 'Semantic analysis is disabled in the server configuration' }, 'Analyze claims'));
  } else {
    body.push(h('p', null, `Enabled${sem.model ? ` with model ${sem.model}` : ''}. `,
      `${sem.queued || 0} queued, ${sem.running || 0} running, ${sem.completed || 0} completed, ${sem.failed || 0} failed.`));
    if (sem.available === false) body.push(h('p', { class: 'warn-line', role: 'note' }, `Currently unavailable${sem.detail ? ': ' + sem.detail : '.'}`));
    else if (sem.detail) body.push(h('p', { class: 'muted' }, sem.detail));
    body.push(h('button', { type: 'button', class: 'btn', disabled: sem.available === false, onclick: onAnalyze }, 'Analyze all claims'));
  }
  return h('section', { class: 'panel', 'aria-labelledby': 'sem-h' }, h('h2', { id: 'sem-h' }, 'Semantic analysis'), body, feedback);
}

function driftList(page) {
  if (!page.items.length) return empty('No outstanding drift.', 'No violated, stale or ambiguous findings are open.');
  return h('ul', { class: 'drift' }, page.items.map((f) => h('li', null,
    h('a', { class: 'drift-row', href: route('findings/' + f.id) },
      statusBadge(f.status), severityBadge(f.severity),
      h('span', { class: 'drift-title' }, f.title),
      h('span', { class: 'mono muted drift-loc' }, f.locations && f.locations[0] ? rangeStr(f.locations[0]) : '')))));
}

export function mount(root, ctx) {
  const summary = h('div', { class: 'stack' });
  const driftBox = h('div', null, loading('Loading findings'));
  const feedback = h('div', { 'aria-live': 'polite' });
  const scanFeedback = h('span', { 'aria-live': 'polite' });
  const rescan = h('button', { type: 'button', class: 'btn btn-primary', onclick: async () => {
    rescan.disabled = true;
    replaceChildren(scanFeedback);
    try { await post('/scans', { force: false }); replaceChildren(scanFeedback, 'Scan requested.'); }
    catch (e) { replaceChildren(scanFeedback, inlineError(e)); }
    rescan.disabled = false;
  } }, 'Rescan');
  const header = pageHeader('Dashboard', rescan, scanFeedback);
  root.append(h('div', { class: 'view' }, header, summary,
    h('section', { class: 'panel', 'aria-labelledby': 'drift-h' }, h('h2', { id: 'drift-h' }, 'Outstanding drift'), driftBox,
      h('p', null, h('a', { href: route('findings', { status: 'violated,stale,ambiguous' }) }, 'All drift findings')))));

  async function analyze() {
    replaceChildren(feedback);
    try {
      const r = await post('/analyses', { claims: [], all: true });
      replaceChildren(feedback, h('p', { class: 'ok-line' }, `${plural(r.queued, 'claim')} queued for analysis.`));
    } catch (e) {
      replaceChildren(feedback, inlineError(e.code === 'conflict' || e.status === 409 ? new Error('Semantic analysis is disabled or unavailable.') : e));
    }
  }

  function paintSummary() {
    const st = getState().status;
    if (!st) return;
    const repo = st.repository || {};
    replaceChildren(summary,
      h('section', { class: 'panel', 'aria-labelledby': 'repo-h' }, h('h2', { id: 'repo-h' }, 'Repository'),
        h('p', { class: 'mono wrap' }, repo.root),
        h('dl', { class: 'kv' }, rootsList('Spec roots', repo.spec_roots), rootsList('Source roots', repo.src_roots)),
        st.counts ? h('p', { class: 'muted' }, `${plural(st.counts.documents, 'document')}, ${plural(st.counts.source_files, 'source file')}, ${plural(st.counts.symbols, 'symbol')}, ${plural(st.counts.edges, 'edge')}; revision ${st.revision}.`) : null),
      h('section', { 'aria-labelledby': 'tiles-h' }, h('h2', { id: 'tiles-h', class: 'sr-only' }, 'Counts'), tiles(st.counts)),
      h('div', { class: 'grid-2' }, scanPanel(st), semanticPanel(st, analyze, feedback)));
  }

  async function loadDrift() {
    try {
      const page = await get('/findings', { status: 'violated,stale,ambiguous', limit: 10, sort: 'severity' }, ctx.signal);
      replaceChildren(driftBox, driftList(page));
    } catch (e) {
      if (!isAbort(e)) replaceChildren(driftBox, errorBox(e, () => { replaceChildren(driftBox, loading()); loadDrift(); }));
    }
  }

  async function refresh() {
    try { await loadStatus(ctx.signal); } catch (e) { if (!isAbort(e)) replaceChildren(summary, errorBox(e, refresh)); }
    loadDrift();
  }

  paintSummary();
  ctx.cleanup(subscribe(paintSummary));
  loadDrift();
  ctx.live(['finding.*', 'scan.completed', 'evidence.*', 'analysis.completed'], loadDrift, 2000);
  ctx.onResync(refresh);
  return { refresh };
}
