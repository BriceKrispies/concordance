import { h, nodeLink } from '../dom.js';
import { methodText, rangeStr, titleCase } from '../format.js';
import { pill } from './badge.js';

function citation(c) {
  return h('li', { class: 'citation' },
    h('div', null, pill(titleCase(c.role)), ' ', h('span', { class: 'mono' }, rangeStr(c.range)),
      c.node ? [' ', nodeLink(c.node, 'node')] : null),
    c.excerpt ? h('pre', { class: 'excerpt-pre' }, c.excerpt) : null);
}

export function evidenceItem(ev, { stanceOverride } = {}) {
  const stance = stanceOverride || ev.stance;
  const invalid = ev.validity && ev.validity.state === 'invalidated';
  return h('article', { class: `evidence ev-${stance}${invalid ? ' ev-invalid' : ''}` },
    h('header', { class: 'ev-head' },
      pill(titleCase(stance), `stance-${stance}`),
      pill(`${ev.strength} strength`),
      pill(methodText(ev.method), 'pill-method'),
      h('span', { class: 'muted' }, `rev ${ev.revision}`)),
    h('p', null, ev.summary),
    invalid
      ? h('p', { class: 'warn-line' }, `Invalidated at revision ${ev.validity.revision}: ${ev.validity.reason}`,
        ev.validity.changed && ev.validity.changed.length
          ? [' Changed: ', ev.validity.changed.map((n, i) => [i ? ', ' : '', nodeLink(n)])] : null)
      : null,
    ev.citations && ev.citations.length ? h('ul', { class: 'plain' }, ev.citations.map(citation)) : null,
    ev.dependencies && ev.dependencies.length
      ? h('p', { class: 'muted small' }, 'Depends on: ', ev.dependencies.map((d, i) => [i ? ', ' : '', nodeLink(d.node)]))
      : null,
    h('p', { class: 'muted small mono' }, ev.id));
}

// Group evidence by whether it supports or contradicts.
export function evidenceGroups(all, finding) {
  const sup = new Set(finding ? finding.supporting : []);
  const con = new Set(finding ? finding.contradicting : []);
  const supporting = [];
  const contradicting = [];
  const other = [];
  for (const ev of all || []) {
    if (con.has(ev.id) || (!sup.has(ev.id) && ev.stance === 'contradicts')) contradicting.push(ev);
    else if (sup.has(ev.id) || ev.stance === 'supports') supporting.push(ev);
    else other.push(ev);
  }
  const block = (title, list) => list.length
    ? h('section', { class: 'ev-group' }, h('h3', null, `${title} (${list.length})`), h('div', { class: 'stack' }, list.map((e) => evidenceItem(e))))
    : null;
  if (!all || !all.length) return h('p', { class: 'muted' }, 'No evidence has been recorded for this finding.');
  return h('div', { class: 'stack' }, block('Contradicting evidence', contradicting), block('Supporting evidence', supporting), block('Inconclusive evidence', other));
}
