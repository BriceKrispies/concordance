import { h, route, nodeLink, replaceChildren } from '../dom.js';
import { get, isAbort } from '../api.js';
import { CATEGORY_TEXT, ACTION_TEXT, pct, rangeStr, when, titleCase, methodText } from '../format.js';
import { statusBadge, severityBadge, pill } from '../components/badge.js';
import { loading, errorBox } from '../components/state.js';
import { excerptList } from '../components/code-viewer.js';
import { evidenceGroups } from '../components/evidence-list.js';

function limitations(f) {
  const semantic = (f.methods || []).some((m) => m === 'semantic' || m === 'adversarial');
  const items = [...(f.limitations || [])];
  if (!items.length && !semantic) return null;
  return h('aside', { class: 'limitations', role: 'note', 'aria-labelledby': 'lim-h' },
    h('h2', { id: 'lim-h' }, 'Limitations'),
    semantic ? h('p', null, h('strong', null, 'Model-assessed. '), 'Part of this finding comes from a language model. Treat its explanation as a lead to check, not as proof.') : null,
    items.length ? h('ul', null, items.map((l) => h('li', null, l))) : null);
}

function facts(d) {
  const f = d.finding;
  const life = f.lifecycle && f.lifecycle.state === 'resolved'
    ? `Resolved at revision ${f.lifecycle.revision} (${when(f.lifecycle.at)})` : 'Open';
  const row = (k, v) => v == null || v === '' ? null : [h('dt', null, k), h('dd', null, v)];
  return h('dl', { class: 'kv kv-grid' },
    row('Category', CATEGORY_TEXT[f.category] || titleCase(f.category)),
    row('Severity', severityBadge(f.severity)),
    row('Confidence', pct(f.confidence)),
    row('Evidence quality', titleCase(f.quality)),
    row('Methods', (f.methods || []).map(titleCase).join(', ') || 'none'),
    row('Lifecycle', life),
    row('Introduced in', `revision ${f.introduced_in}${f.introduced_by && f.introduced_by.length ? ' by ' + f.introduced_by.join(', ') : ''}`),
    row('Last evaluated at', `revision ${f.revision}`),
    row('First seen', when(f.first_seen)),
    row('Last updated', when(f.last_updated)),
    row('Crate / module', f.crate_name ? `${f.crate_name}${f.module ? ' :: ' + f.module : ''}` : null));
}

function links(d) {
  const f = d.finding;
  const items = [];
  for (const c of d.claims || []) items.push(h('li', null, 'Claim ', h('a', { href: route('claims/' + c.claim.id) }, c.claim.text.length > 90 ? c.claim.text.slice(0, 90) + '…' : c.claim.text),
    ' ', statusBadge(c.status), ' ', h('a', { class: 'small', href: route('graph/' + c.claim.id) }, 'graph')));
  for (const s of d.symbols || []) items.push(h('li', null, 'Symbol ', h('a', { class: 'mono', href: route('code/' + s.id) }, s.qualified_name),
    ' ', h('a', { class: 'small', href: route('graph/' + s.id) }, 'graph')));
  for (const doc of f.documents || []) items.push(h('li', null, 'Document ', nodeLink(doc)));
  if (!items.length) return null;
  return h('section', { class: 'panel', 'aria-labelledby': 'links-h' }, h('h2', { id: 'links-h' }, 'Related'), h('ul', { class: 'plain' }, items));
}

function proposals(f) {
  if (!f.proposals || !f.proposals.length) return null;
  return h('section', { class: 'panel', 'aria-labelledby': 'prop-h' }, h('h2', { id: 'prop-h' }, 'Proposals'),
    h('ul', { class: 'plain stack' }, f.proposals.map((p) => h('li', null,
      h('strong', null, ACTION_TEXT[p.action] || titleCase(p.action)),
      p.requires_decision ? pill('Requires decision', 'pill-warn') : null,
      h('p', null, p.rationale),
      p.targets && p.targets.length ? h('p', { class: 'mono small muted' }, p.targets.map(rangeStr).join(', ')) : null))));
}

function history(d) {
  const list = d.history || [];
  return h('section', { class: 'panel', 'aria-labelledby': 'hist-h' }, h('h2', { id: 'hist-h' }, 'History'),
    list.length
      ? h('ol', { class: 'timeline' }, [...list].reverse().map((t) => h('li', null,
        h('span', { class: 'muted small' }, `${when(t.at)} (revision ${t.revision})`), ' ',
        t.from ? statusBadge(t.from) : h('span', { class: 'muted' }, 'new'), ' → ',
        t.to ? statusBadge(t.to) : h('span', { class: 'muted' }, 'resolved'),
        h('div', { class: 'small' }, t.title))))
      : h('p', { class: 'muted' }, 'No recorded transitions.'));
}

function disagree(d) {
  const panes = [
    ['spec', 'Spec', excerptList(d.spec_excerpts, 'Spec', 'No specification passage is available.')],
    ['code', 'Code', excerptList(d.code_excerpts, 'Code', 'No code excerpt is available.')],
    ['evidence', 'Evidence', evidenceGroups(d.evidence, d.finding)],
  ];
  const tabs = h('div', { class: 'tabs', role: 'tablist', 'aria-label': 'What disagrees' });
  const paneEls = {};
  const tabEls = {};
  function select(key) {
    for (const [k] of panes) {
      const on = k === key;
      paneEls[k].classList.toggle('active', on);
      tabEls[k].setAttribute('aria-selected', String(on));
      tabEls[k].tabIndex = on ? 0 : -1;
    }
  }
  panes.forEach(([key, label], i) => {
    tabEls[key] = h('button', { type: 'button', role: 'tab', class: 'tab', id: `tab-${key}`, 'aria-controls': `pane-${key}`, onclick: () => select(key),
      onkeydown: (e) => {
        const dir = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
        if (!dir) return;
        const next = panes[(i + dir + panes.length) % panes.length][0];
        select(next); tabEls[next].focus();
      } }, label);
    tabs.append(tabEls[key]);
  });
  const wrap = h('div', { class: 'panes' }, panes.map(([key, label, content]) => {
    paneEls[key] = h('section', { class: 'pane', id: `pane-${key}`, role: 'tabpanel', 'aria-labelledby': `tab-${key}` }, h('h3', { class: 'pane-title' }, label === 'Evidence' ? 'Evidence' : label === 'Spec' ? 'Specification' : 'Code'), content);
    return paneEls[key];
  }));
  select('spec');
  return h('section', { class: 'disagree', 'aria-labelledby': 'dis-h' }, h('h2', { id: 'dis-h' }, 'What disagrees'), tabs, wrap);
}

function render(d) {
  const f = d.finding;
  return h('article', { class: 'stack' },
    h('header', { class: 'detail-head' },
      h('div', { class: 'row' }, statusBadge(f.status), severityBadge(f.severity), pill(f.lifecycle && f.lifecycle.state === 'resolved' ? 'Resolved' : 'Open')),
      h('h1', null, f.title),
      h('p', { class: 'category' }, CATEGORY_TEXT[f.category] || titleCase(f.category))),
    limitations(f),
    h('section', { class: 'panel', 'aria-labelledby': 'exp-h' }, h('h2', { id: 'exp-h' }, 'Explanation'),
      h('p', { class: 'prewrap' }, f.explanation),
      (f.methods || []).length ? h('p', { class: 'muted small' }, 'Evidence methods: ', (d.evidence || []).map((e) => methodText(e.method)).filter((v, i, a) => a.indexOf(v) === i).join('; ') || (f.methods || []).join(', ')) : null),
    disagree(d),
    h('div', { class: 'grid-2' }, h('section', { class: 'panel', 'aria-labelledby': 'det-h' }, h('h2', { id: 'det-h' }, 'Details'), facts(d)), links(d)),
    proposals(f),
    history(d));
}

export function mount(root, ctx) {
  const id = ctx.params.id;
  const body = h('div', null, loading('Loading finding'));
  root.append(h('div', { class: 'view' }, h('p', { class: 'crumbs' }, h('a', { href: route('findings') }, '← Findings')), body));
  async function load(quiet) {
    if (!quiet) replaceChildren(body, loading('Loading finding'));
    try {
      const d = await get('/findings/' + encodeURIComponent(id), null, ctx.signal);
      replaceChildren(body, render(d));
    } catch (e) {
      if (isAbort(e)) return;
      replaceChildren(body, e.status === 404
        ? h('div', { class: 'state state-empty' }, h('p', null, 'This finding no longer exists. It may have been resolved and removed.'), h('a', { href: route('findings') }, 'Back to findings'))
        : errorBox(e, () => load()));
    }
  }
  load();
  ctx.live(['finding.*', 'evidence.*'], () => load(true), 2000);
  ctx.onResync(() => load(true));
  return { refresh: () => load(true) };
}
