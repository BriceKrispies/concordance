// Filter form for the findings explorer. Mutates the shared query object and calls onChange.
import { h, debounce } from '../dom.js';
import { STATUSES, SEVERITIES, titleCase } from '../format.js';

const csv = (v) => (v ? v.split(',').filter(Boolean) : []);

function checkGroup(legend, name, options, q, onChange) {
  const selected = new Set(csv(q[name]));
  return h('fieldset', { class: 'fs' }, h('legend', null, legend),
    h('div', { class: 'checks' }, options.map((o) => h('label', { class: 'check' },
      h('input', { type: 'checkbox', value: o, checked: selected.has(o), onchange: (e) => {
        if (e.target.checked) selected.add(o); else selected.delete(o);
        q[name] = options.filter((x) => selected.has(x)).join(',');
        onChange();
      } }), h('span', null, o)))));
}

function select(label, name, options, q, onChange) {
  const sel = h('select', { id: 'f-' + name, onchange: (e) => { q[name] = e.target.value; onChange(); } },
    options.map(([value, text]) => h('option', { value, selected: (q[name] || '') === value }, text)));
  return h('div', { class: 'field' }, h('label', { for: 'f-' + name }, label), sel);
}

export function buildFilters(q, catalog, onChange, onReset) {
  const search = h('input', { id: 'f-q', type: 'search', value: q.q || '', placeholder: 'Title, explanation, claim, symbol', autocomplete: 'off' });
  const debounced = debounce(() => { q.q = search.value.trim(); onChange(); }, 300);
  search.addEventListener('input', debounced);

  const docs = [['', 'All documents'], ...catalog.documents.map((d) => [d.id, d.title ? `${d.title} (${d.path})` : d.path])];
  const crates = [['', 'All crates'], ...catalog.crates.map((c) => [c.name, c.name])];

  const form = h('form', { class: 'filters', onsubmit: (e) => { e.preventDefault(); debounced.cancel(); q.q = search.value.trim(); onChange(); } },
    h('div', { class: 'field field-wide' }, h('label', { for: 'f-q' }, 'Search'), search),
    checkGroup('Status', 'status', STATUSES, q, onChange),
    checkGroup('Severity', 'severity', SEVERITIES, q, onChange),
    h('div', { class: 'filter-row' },
      select('Method', 'method', [['', 'Any'], ['structural', 'Structural'], ['semantic', 'Semantic'], ['adversarial', 'Adversarial'], ['test', 'Test']], q, onChange),
      select('Lifecycle', 'lifecycle', [['', 'Open'], ['resolved', 'Resolved'], ['all', 'All']], q, onChange),
      select('Document', 'document', docs, q, onChange),
      select('Crate', 'crate', crates, q, onChange),
      select('Sort', 'sort', [['', 'Severity'], ['updated', 'Recently updated'], ['status', 'Status'], ['title', 'Title']], q, onChange)),
    h('div', { class: 'filter-row' },
      h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: q.new === 'true', onchange: (e) => { q.new = e.target.checked ? 'true' : ''; onChange(); } }),
        h('span', null, 'New in latest change')),
      h('button', { type: 'button', class: 'btn', onclick: onReset }, 'Clear filters')));
  return form;
}

export function describeFilters(q) {
  const parts = [];
  for (const k of ['status', 'severity', 'method', 'lifecycle', 'document', 'crate', 'q']) if (q[k]) parts.push(`${titleCase(k)}: ${q[k]}`);
  if (q.new === 'true') parts.push('New in latest change');
  return parts;
}
