import { h, route, replaceChildren } from '../dom.js';
import { get, isAbort } from '../api.js';
import { loadCatalog } from '../store.js';
import { replaceHash } from '../router.js';
import { pct, rangeStr } from '../format.js';
import { statusBadge, severityBadge, methodPills } from '../components/badge.js';
import { loading, empty, errorBox, pageHeader } from '../components/state.js';
import { buildFilters, describeFilters } from '../components/finding-filters.js';
import { pager } from '../components/pagination.js';
import { subscribe as onEvent } from '../events.js';

const LIMIT = 50;
const KEYS = ['status', 'severity', 'method', 'lifecycle', 'new', 'document', 'crate', 'q', 'sort', 'offset'];

function loc(f) {
  return f.locations && f.locations[0] ? rangeStr(f.locations[0]) : '';
}

function card(f, flash) {
  return h('li', { class: `fcard${flash ? ' flash' : ''}`, dataset: { id: f.id } },
    h('a', { class: 'fcard-link', href: route('findings/' + f.id) },
      h('div', { class: 'fcard-top' }, statusBadge(f.status), severityBadge(f.severity), h('span', { class: 'muted small' }, pct(f.confidence))),
      h('div', { class: 'fcard-title' }, f.title),
      h('div', { class: 'mono small muted wrap' }, loc(f)),
      h('div', { class: 'fcard-meta' }, methodPills(f.methods))));
}

function table(items, flashIds) {
  return h('table', { class: 'dense' },
    h('thead', null, h('tr', null, ['Status', 'Severity', 'Title', 'Location', 'Methods', 'Confidence'].map((c) => h('th', { scope: 'col' }, c)))),
    h('tbody', null, items.map((f) => h('tr', { class: flashIds.has(f.id) ? 'flash' : '', dataset: { id: f.id } },
      h('td', null, statusBadge(f.status)),
      h('td', null, severityBadge(f.severity)),
      h('td', { class: 'td-title' }, h('a', { href: route('findings/' + f.id) }, f.title)),
      h('td', { class: 'mono small wrap' }, loc(f)),
      h('td', null, methodPills(f.methods)),
      h('td', null, pct(f.confidence))))));
}

export function mount(root, ctx) {
  const q = {};
  for (const k of KEYS) if (ctx.query[k]) q[k] = ctx.query[k];
  let offset = Number(q.offset) || 0;
  let flashIds = new Set();
  let controller = null;
  ctx.signal.addEventListener('abort', () => controller && controller.abort(), { once: true });

  const results = h('div', { 'aria-live': 'polite' });
  const summary = h('p', { class: 'muted' });
  const filterHost = h('details', { class: 'filter-panel' }, h('summary', null, 'Filters'));
  filterHost.open = window.matchMedia('(min-width: 900px)').matches;
  let catalog = { documents: [], crates: [] };

  root.append(h('div', { class: 'view' }, pageHeader('Findings'), filterHost, summary, results));

  function syncHash() {
    const query = {};
    for (const k of KEYS) if (q[k]) query[k] = q[k];
    if (offset) query.offset = offset;
    replaceHash(route('findings', query));
  }

  function paintForm() {
    const form = buildFilters(q, catalog, () => { offset = 0; syncHash(); load(); },
      () => { for (const k of KEYS) delete q[k]; offset = 0; syncHash(); paintForm(); load(); });
    replaceChildren(filterHost, h('summary', null, 'Filters', describeFilters(q).length ? ` (${describeFilters(q).length} active)` : ''), form);
  }

  async function load(opts = {}) {
    if (controller) controller.abort();
    controller = new AbortController();
    const signal = controller.signal;
    if (!opts.quiet) replaceChildren(results, loading('Loading findings'));
    try {
      const page = await get('/findings', { ...q, offset: offset || undefined, limit: LIMIT }, signal);
      // Server may clamp offset when results shrink.
      if (page.items.length === 0 && page.total > 0 && offset > 0) { offset = 0; syncHash(); return load(); }
      summary.textContent = `${page.total} finding${page.total === 1 ? '' : 's'}`;
      if (!page.items.length) {
        replaceChildren(results, empty('No findings match these filters.', 'Adjust or clear the filters to see more.'));
      } else {
        replaceChildren(results,
          h('ul', { class: 'fcards plain' }, page.items.map((f) => card(f, flashIds.has(f.id)))),
          table(page.items, flashIds),
          pager({ total: page.total, offset: page.offset ?? offset, limit: page.limit || LIMIT }, (o) => { offset = o; syncHash(); load(); window.scrollTo(0, 0); }));
        if (flashIds.size) setTimeout(() => { for (const el of results.querySelectorAll('.flash')) el.classList.remove('flash'); }, 3000);
      }
      flashIds = new Set();
    } catch (e) {
      if (!isAbort(e)) replaceChildren(results, errorBox(e, () => load()));
    }
  }

  loadCatalog(ctx.signal).then((c) => { catalog = c; paintForm(); }, () => { paintForm(); });
  paintForm();
  load();

  // Live: mark changed rows, refresh the current page at most every 2 s.
  collectFlash(ctx, (id) => flashIds.add(id));
  ctx.live(['finding.*'], () => load({ quiet: true }), 2000);
  ctx.onResync(() => load());
  return { refresh: () => load({ quiet: true }), destroy: () => controller && controller.abort() };
}

// Remember which findings changed so the next refresh can highlight them.
function collectFlash(ctx, add) {
  ctx.cleanup(onEvent('*', (env) => {
    if (!env.type.startsWith('finding.')) return;
    const d = env.data || {};
    const id = env.type === 'finding.resolved' ? d.finding : d.finding && d.finding.id;
    if (id) add(id);
  }));
}
