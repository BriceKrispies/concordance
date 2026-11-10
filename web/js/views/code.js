import { h, route, replaceChildren, debounce } from '../dom.js';
import { get, isAbort } from '../api.js';
import { loadCatalog } from '../store.js';
import { replaceHash } from '../router.js';
import { titleCase, rangeStr, lineSpan } from '../format.js';
import { statusBadge, severityBadge, pill } from '../components/badge.js';
import { loading, empty, errorBox, pageHeader } from '../components/state.js';
import { pager } from '../components/pagination.js';
import { excerptView } from '../components/code-viewer.js';

const LIMIT = 50;
const KINDS = ['module', 'function', 'method', 'associated_function', 'struct', 'enum', 'variant', 'union', 'trait', 'trait_method', 'impl', 'const', 'static', 'type_alias', 'macro_rules'];
const KEYS = ['crate', 'module', 'kind', 'public', 'unconstrained', 'q', 'offset'];

function noContract(sv) {
  return sv.is_public_api && !sv.constrained_by;
}

function symbolRow(sv) {
  return h('li', { class: `sym-row${noContract(sv) ? ' sym-nocontract' : ''}` },
    h('div', { class: 'row' }, pill(titleCase(sv.kind), 'pill-muted'), sv.is_public_api ? pill('public API') : pill(sv.visibility, 'pill-muted'),
      sv.finding_status ? statusBadge(sv.finding_status) : null,
      noContract(sv) ? pill('no documented contract', 'pill-undocumented') : null),
    h('a', { class: 'mono sym-name', href: route('code/' + sv.id) }, sv.qualified_name),
    h('div', { class: 'mono small muted wrap' }, rangeStr(sv.range && { ...sv.range, path: sv.file_path || sv.range.path }),
      sv.constrained_by ? ` · constrained by ${sv.constrained_by} claim${sv.constrained_by === 1 ? '' : 's'}` : ''));
}

function crateCard(c, onPick) {
  return h('li', { class: 'crate-card' },
    h('button', { type: 'button', class: 'crate-btn', onclick: () => onPick(c.name) },
      h('strong', null, c.name),
      h('span', { class: 'muted small' }, `${c.files} files, ${c.symbols} symbols`),
      h('span', { class: 'small' }, `${c.public_api} public API`),
      c.unconstrained_public_api ? h('span', { class: 'small warn-text' }, `${c.unconstrained_public_api} with no documented contract`) : h('span', { class: 'small muted' }, 'all public API constrained')));
}

export function mountList(root, ctx) {
  const q = {};
  for (const k of KEYS) if (ctx.query[k]) q[k] = ctx.query[k];
  let offset = Number(q.offset) || 0;
  let catalog = { crates: [], documents: [] };
  let controller = null;
  ctx.signal.addEventListener('abort', () => controller && controller.abort(), { once: true });
  const cratesBox = h('div', null);
  const formBox = h('div', null);
  const results = h('div', { 'aria-live': 'polite' });
  root.append(h('div', { class: 'view' }, pageHeader('Code'),
    h('section', { 'aria-labelledby': 'crates-h' }, h('h2', { id: 'crates-h' }, 'Crates'), cratesBox),
    h('section', { 'aria-labelledby': 'syms-h' }, h('h2', { id: 'syms-h' }, 'Symbols'), formBox, results)));

  function syncHash() {
    const query = {};
    for (const k of KEYS) if (q[k]) query[k] = q[k];
    if (offset) query.offset = offset;
    replaceHash(route('code', query));
  }
  const changed = () => { offset = 0; syncHash(); load(); };

  function paintForm() {
    const cr = catalog.crates.find((c) => c.name === q.crate);
    const modules = cr ? cr.modules || [] : [...new Set(catalog.crates.flatMap((c) => c.modules || []))];
    const sel = (id, label, opts, key) => h('div', { class: 'field' }, h('label', { for: id }, label),
      h('select', { id, onchange: (e) => { q[key] = e.target.value; if (key === 'crate') delete q.module; changed(); if (key === 'crate') paintForm(); } },
        opts.map(([v, t]) => h('option', { value: v, selected: (q[key] || '') === v }, t))));
    const search = h('input', { id: 'sym-q', type: 'search', value: q.q || '', placeholder: 'Symbol path', autocomplete: 'off' });
    search.addEventListener('input', debounce(() => { q.q = search.value.trim(); changed(); }, 300));
    const chk = (label, key) => h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: q[key] === 'true', onchange: (e) => { q[key] = e.target.checked ? 'true' : ''; changed(); } }), h('span', null, label));
    replaceChildren(formBox, h('form', { class: 'filters', onsubmit: (e) => e.preventDefault() },
      h('div', { class: 'field field-wide' }, h('label', { for: 'sym-q' }, 'Search'), search),
      h('div', { class: 'filter-row' },
        sel('sym-crate', 'Crate', [['', 'All crates'], ...catalog.crates.map((c) => [c.name, c.name])], 'crate'),
        sel('sym-module', 'Module', [['', 'All modules'], ...modules.map((m) => [m, m])], 'module'),
        sel('sym-kind', 'Kind', [['', 'Any kind'], ...KINDS.map((k) => [k, titleCase(k)])], 'kind')),
      h('div', { class: 'filter-row' }, chk('Public API only', 'public'), chk('No documented contract only', 'unconstrained'))));
  }

  function paintCrates() {
    replaceChildren(cratesBox, catalog.crates.length
      ? h('ul', { class: 'crate-grid plain' }, catalog.crates.map((c) => crateCard(c, (name) => { q.crate = name; delete q.module; changed(); paintForm(); })))
      : empty('No crates found.', 'Source roots may be empty or still indexing.'));
  }

  async function load(quiet) {
    if (controller) controller.abort();
    controller = new AbortController();
    if (!quiet) replaceChildren(results, loading('Loading symbols'));
    try {
      const page = await get('/symbols', { ...q, offset: offset || undefined, limit: LIMIT }, controller.signal);
      replaceChildren(results, h('p', { class: 'muted' }, `${page.total} symbol${page.total === 1 ? '' : 's'}`),
        page.items.length ? [h('ul', { class: 'plain sym-list' }, page.items.map(symbolRow)),
          pager({ total: page.total, offset: page.offset ?? offset, limit: page.limit || LIMIT }, (o) => { offset = o; syncHash(); load(); window.scrollTo(0, 0); })]
          : empty('No symbols match these filters.'));
    } catch (e) {
      if (!isAbort(e)) replaceChildren(results, errorBox(e, () => load()));
    }
  }

  async function loadCrates() {
    try { catalog = await loadCatalog(ctx.signal); } catch (e) { if (!isAbort(e)) replaceChildren(cratesBox, errorBox(e, loadCrates)); return; }
    paintCrates(); paintForm();
  }
  replaceChildren(cratesBox, loading('Loading crates'));
  paintForm();
  loadCrates();
  load();
  ctx.live(['scan.completed', 'symbol.*', 'finding.*', 'source.*'], () => { load(true); }, 3000);
  ctx.onResync(() => { loadCrates(); load(true); });
  return { refresh: () => load(true) };
}

// ---- Symbol view ---------------------------------------------------------------------------

export function mountSymbol(root, ctx) {
  const body = h('div', null, loading('Loading symbol'));
  root.append(h('div', { class: 'view' }, h('p', { class: 'crumbs' }, h('a', { href: route('code') }, '← Code')), body));

  function render(d) {
    const s = d.symbol;
    const constrained = d.constrained_by || [];
    const publicNone = s.is_public_api && !constrained.length;
    return h('div', { class: 'stack' },
      h('header', { class: 'detail-head' },
        h('div', { class: 'row' }, pill(titleCase(s.kind), 'pill-muted'), pill(s.visibility, 'pill-muted'), s.is_public_api ? pill('public API') : null,
          publicNone ? pill('no documented contract', 'pill-undocumented') : null),
        h('h1', { class: 'mono wrap' }, (s.path || []).join('::')),
        h('p', { class: 'mono small muted wrap' }, d.file ? `${d.file.path}` : '', ' ', lineSpan(s.range)),
        h('p', { class: 'row small' }, h('a', { href: route('graph/' + s.id) }, 'Graph'))),
      s.signature ? h('section', { class: 'panel' }, h('h2', null, 'Signature'), h('pre', { class: 'excerpt-pre' }, s.signature)) : null,
      s.doc ? h('section', { class: 'panel' }, h('h2', null, 'Documentation comment'), h('pre', { class: 'excerpt-pre prewrap' }, s.doc)) : null,
      (s.cfg && s.cfg.length) || (s.attributes && s.attributes.length) ? h('section', { class: 'panel' }, h('h2', null, 'Configuration'),
        s.cfg.length ? h('p', null, 'cfg: ', s.cfg.map((c) => h('code', { class: 'chip' }, c))) : null,
        s.attributes.length ? h('p', null, 'attributes: ', s.attributes.map((c) => h('code', { class: 'chip' }, c))) : null) : null,
      h('section', { class: 'panel', 'aria-labelledby': 'con-h' }, h('h2', { id: 'con-h' }, 'Constrained by'),
        constrained.length
          ? h('ul', { class: 'plain stack-sm' }, constrained.map((c) => h('li', null, statusBadge(c.claim.status), ' ',
            h('a', { href: route('claims/' + c.claim.claim.id) }, c.claim.claim.text),
            h('span', { class: 'muted small' }, ` (${titleCase(c.edge.kind)}, ${(c.edge.resolution && c.edge.resolution.kind) || ''})`))))
          : h('p', { class: publicNone ? 'warn-text' : 'muted' }, publicNone ? 'No documented contract: no specification claim constrains this public API.' : 'No claims constrain this symbol.')),
      d.findings && d.findings.length ? h('section', { class: 'panel' }, h('h2', null, 'Findings'),
        h('ul', { class: 'plain stack-sm' }, d.findings.map((f) => h('li', null, statusBadge(f.status), ' ', severityBadge(f.severity), ' ', h('a', { href: route('findings/' + f.id) }, f.title))))) : null,
      s.facts && s.facts.length ? h('section', { class: 'panel' }, h('h2', null, 'Facts extracted from code'),
        h('table', { class: 'dense facts' }, h('thead', null, h('tr', null, ['Kind', 'Line', 'Detail'].map((c) => h('th', { scope: 'col' }, c)))),
          h('tbody', null, s.facts.map((f) => h('tr', null, h('td', null, titleCase(f.kind)), h('td', { class: 'mono' }, f.range.start_line), h('td', { class: 'mono wrap' }, f.detail)))))) : null,
      d.children && d.children.length ? h('section', { class: 'panel' }, h('h2', null, `Children (${d.children.length})`), h('ul', { class: 'plain sym-list' }, d.children.map(symbolRow))) : null,
      d.excerpt ? h('section', { class: 'panel' }, h('h2', null, 'Source'), excerptView(d.excerpt, { label: 'Code' })) : null);
  }

  async function load(quiet) {
    if (!quiet) replaceChildren(body, loading('Loading symbol'));
    try {
      replaceChildren(body, render(await get('/symbols/' + encodeURIComponent(ctx.params.id), null, ctx.signal)));
    } catch (e) {
      if (!isAbort(e)) replaceChildren(body, e.status === 404 ? empty('Symbol not found.', 'It may have been removed or renamed.') : errorBox(e, () => load()));
    }
  }
  load();
  ctx.live(['finding.*', 'scan.completed', 'symbol.*'], () => load(true), 3000);
  ctx.onResync(() => load(true));
  return { refresh: () => load(true) };
}
