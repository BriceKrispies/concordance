import { h, route, nodeLink, replaceChildren, debounce } from '../dom.js';
import { get, isAbort } from '../api.js';
import { titleCase, rangeStr, pct, resolutionText, referenceTargetText, STATUSES } from '../format.js';
import { statusBadge, pill } from '../components/badge.js';
import { loading, empty, errorBox, pageHeader } from '../components/state.js';
import { evidenceItem } from '../components/evidence-list.js';

function authorityText(a) {
  if (!a) return 'authority not established';
  if (a.level === 'unknown') return 'authority not established';
  return `${titleCase(a.level)} authority`;
}

function authorityBlock(a) {
  return h('span', { class: 'authority' }, h('strong', null, authorityText(a)),
    a && a.level !== 'unknown' && a.rationale ? h('span', { class: 'muted' }, ` - ${a.rationale}`) : null,
    a && a.level === 'unknown' && a.rationale ? h('span', { class: 'muted' }, ` - ${a.rationale}`) : null);
}

function loader(body, ctx, fetcher, render) {
  async function load(quiet) {
    if (!quiet) replaceChildren(body, loading());
    try {
      replaceChildren(body, render(await fetcher(ctx.signal)));
    } catch (e) {
      if (isAbort(e)) return;
      replaceChildren(body, e.status === 404 ? empty('Not found.', 'It may have been removed by a recent change.') : errorBox(e, () => load()));
    }
  }
  load();
  return load;
}

// ---- Document list -------------------------------------------------------------------------

function docCard(d) {
  const sc = d.status_counts || {};
  return h('li', { class: 'doc-card' },
    h('h2', null, h('a', { href: route('specs/' + d.id) }, d.title || d.path)),
    h('p', { class: 'mono small muted wrap' }, d.path),
    h('p', { class: 'small' }, authorityBlock(d.authority)),
    h('p', { class: 'small muted' }, `${d.sections} sections, ${d.claims} claims, ${d.obligations} obligations`),
    h('div', { class: 'row' }, STATUSES.filter((s) => sc[s]).map((s) =>
      h('a', { class: `badge badge-${s}`, href: route('findings', { status: s, document: d.id }), title: `Show ${s} findings in this document` }, `${s} ${sc[s]}`))));
}

export function mountList(root, ctx) {
  const body = h('div', null);
  const search = h('input', { type: 'search', id: 'doc-q', placeholder: 'Filter documents', autocomplete: 'off' });
  let q = '';
  root.append(h('div', { class: 'view' }, pageHeader('Specs'),
    h('div', { class: 'field field-wide' }, h('label', { for: 'doc-q' }, 'Search documents'), search), body));
  const load = loader(body, ctx, (signal) => get('/documents', { q, limit: 200 }, signal), (page) =>
    page.items.length
      ? h('ul', { class: 'doc-grid plain' }, page.items.map(docCard))
      : empty('No specification documents found.', 'Add Markdown files under a configured spec root.'));
  search.addEventListener('input', debounce(() => { q = search.value.trim(); load(); }, 300));
  ctx.live(['finding.*', 'document.*', 'scan.completed'], () => load(true), 3000);
  ctx.onResync(() => load(true));
  return { refresh: () => load(true) };
}

// ---- Document view -------------------------------------------------------------------------

function claimRow(cv) {
  const c = cv.claim;
  return h('li', { class: 'claim-row' },
    h('div', { class: 'row' }, statusBadge(cv.status), pill(titleCase(c.kind)), pill(titleCase(c.modality)), pill(titleCase(c.authority), 'pill-muted'),
      h('span', { class: 'mono small muted' }, `L${c.range.start_line}${c.range.end_line !== c.range.start_line ? '-' + c.range.end_line : ''}`)),
    h('a', { class: 'claim-text', href: route('claims/' + c.id) }, c.text));
}

function sectionTree(detail) {
  const bySection = new Map();
  for (const cv of detail.claims) {
    const k = cv.claim.section;
    if (!bySection.has(k)) bySection.set(k, []);
    bySection.get(k).push(cv);
  }
  const children = new Map();
  const ids = new Set(detail.sections.map((s) => s.id));
  for (const s of detail.sections) {
    const parent = s.parent && ids.has(s.parent) ? s.parent : null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(s);
  }
  function node(sec, depth) {
    const claims = bySection.get(sec.id) || [];
    const kids = children.get(sec.id) || [];
    return h('details', { class: 'sec', open: depth < 2 },
      h('summary', null, h('span', { class: 'sec-h' }, sec.heading || '(untitled)'),
        sec.role && sec.role !== 'body' ? pill(titleCase(sec.role), 'pill-muted') : null,
        h('span', { class: 'muted small' }, ` ${claims.length} claim${claims.length === 1 ? '' : 's'}`)),
      claims.length ? h('ul', { class: 'plain' }, claims.map(claimRow)) : null,
      kids.map((k) => node(k, depth + 1)));
  }
  const roots = children.get(null) || [];
  return roots.length ? roots.map((r) => node(r, 0)) : empty('This document has no sections.');
}

export function mountDocument(root, ctx) {
  const body = h('div', null);
  root.append(h('div', { class: 'view' }, h('p', { class: 'crumbs' }, h('a', { href: route('specs') }, '← Specs')), body));
  const load = loader(body, ctx, (signal) => get('/documents/' + encodeURIComponent(ctx.params.id), null, signal), (d) => {
    const doc = d.document;
    const refs = d.prose_references || [];
    return h('div', { class: 'stack' },
      h('header', { class: 'detail-head' }, h('h1', null, doc.title || doc.path), h('p', { class: 'mono muted wrap' }, doc.path),
        h('p', null, authorityBlock(doc.authority)),
        h('p', { class: 'row small' }, h('a', { href: route('graph/' + doc.id) }, 'Graph'),
          h('a', { href: route('findings', { document: doc.id, lifecycle: 'all' }) }, 'Findings'))),
      h('section', { 'aria-label': 'Sections and claims' }, sectionTree(d)),
      refs.length ? h('details', { class: 'panel' }, h('summary', null, `Prose references (${refs.length})`),
        h('ul', { class: 'plain' }, refs.map((r) => h('li', { class: 'small' }, h('span', { class: 'mono' }, r.reference.text), ' → ',
          h('span', { class: 'mono' }, referenceTargetText(r.reference.target) || r.reference.target.kind), ' ', pill(r.reference.strength, 'pill-muted'))))) : null);
  });
  ctx.live(['finding.*', 'claim.*', 'scan.completed'], () => load(true), 3000);
  ctx.onResync(() => load(true));
  return { refresh: () => load(true) };
}

// ---- Claim view ----------------------------------------------------------------------------

function edgeRow(ev) {
  const e = ev.edge;
  const weak = ['lexical', 'ambiguous', 'unresolved'].includes(e.resolution && e.resolution.kind);
  return h('li', { class: 'edge-row' },
    pill(titleCase(e.kind), 'pill-method'),
    h('span', { class: 'mono' }, e.target_text), ' → ',
    e.to ? nodeLink(e.to, ev.target_label || e.to) : h('span', { class: 'muted' }, 'unresolved'),
    ev.target_kind ? h('span', { class: 'muted small' }, ` (${ev.target_kind})`) : null,
    ' ', pill(resolutionText(e.resolution), weak ? 'pill-warn' : 'pill-muted'),
    h('span', { class: 'muted small' }, ` confidence ${pct(e.confidence)}`));
}

export function mountClaim(root, ctx) {
  const body = h('div', null);
  root.append(h('div', { class: 'view' }, h('p', { class: 'crumbs' }, h('a', { href: route('specs') }, '← Specs')), body));
  const load = loader(body, ctx, (signal) => get('/claims/' + encodeURIComponent(ctx.params.id), null, signal), (d) => {
    const c = d.claim;
    return h('div', { class: 'stack' },
      h('header', { class: 'detail-head' },
        h('div', { class: 'row' }, statusBadge(d.status), pill(titleCase(c.kind)), pill(titleCase(c.modality)), pill(titleCase(c.authority), 'pill-muted')),
        h('blockquote', { class: 'claim-quote' }, c.text),
        h('p', { class: 'small muted' }, 'In ', h('a', { href: route('specs/' + d.document.id) }, d.document.title || d.document.path),
          ' at ', h('span', { class: 'mono' }, rangeStr(c.range)), c.heading_path && c.heading_path.length ? ` under "${c.heading_path.join(' > ')}"` : ''),
        h('p', { class: 'row small' }, h('a', { href: route('graph/' + c.id) }, 'Graph'),
          d.finding ? h('a', { href: route('findings/' + d.finding.id) }, 'Finding: ' + d.finding.title) : null)),
      c.ambiguity && c.ambiguity.length ? h('section', { class: 'panel' }, h('h2', null, 'Ambiguity notes'),
        h('ul', null, c.ambiguity.map((a) => h('li', null, h('strong', null, titleCase(a.kind) + ': '), a.detail)))) : null,
      c.documented_signature || (c.bounds && c.bounds.length) || c.constraint ? h('section', { class: 'panel' }, h('h2', null, 'Extracted structure'),
        c.documented_signature ? h('p', null, 'Documented signature: ', h('code', null, c.documented_signature)) : null,
        c.bounds && c.bounds.length ? h('ul', null, c.bounds.map((b) => h('li', null, b.text || `${b.subject} ${b.op} ${b.value}${b.unit ? ' ' + b.unit : ''}`))) : null,
        c.constraint ? h('p', null, 'Constraint: ', h('code', null, JSON.stringify(c.constraint))) : null) : null,
      h('section', { class: 'panel', 'aria-labelledby': 'refs-h' }, h('h2', { id: 'refs-h' }, 'References'),
        c.references && c.references.length
          ? h('ul', { class: 'plain' }, c.references.map((r) => h('li', null, h('span', { class: 'mono' }, r.text), ' → ',
            h('span', { class: 'mono' }, referenceTargetText(r.target)), ' ', pill(r.strength, 'pill-muted'), ' ', pill(titleCase(r.target.kind), 'pill-muted'))))
          : h('p', { class: 'muted' }, 'This claim references no code, files or sections.')),
      h('section', { class: 'panel', 'aria-labelledby': 'edges-h' }, h('h2', { id: 'edges-h' }, 'Relationships'),
        d.edges && d.edges.length ? h('ul', { class: 'plain stack-sm' }, d.edges.map(edgeRow)) : h('p', { class: 'muted' }, 'No relationships recorded.')),
      h('section', { class: 'panel', 'aria-labelledby': 'cev-h' }, h('h2', { id: 'cev-h' }, 'Evidence'),
        d.evidence && d.evidence.length ? h('div', { class: 'stack' }, d.evidence.map((e) => evidenceItem(e))) : h('p', { class: 'muted' }, 'No evidence recorded.')));
  });
  ctx.live(['finding.*', 'evidence.*', 'claim.*'], () => load(true), 3000);
  ctx.onResync(() => load(true));
  return { refresh: () => load(true) };
}
