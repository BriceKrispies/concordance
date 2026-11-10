// Plain-text viewer for SourceExcerpt: line numbers, highlighted range, stale-file warning.
// Markdown is deliberately shown as preformatted text, never rendered.
import { h } from '../dom.js';

export function excerptView(ex, { label } = {}) {
  const hl = ex.highlight;
  const first = ex.start_line;
  const rows = (ex.lines || []).map((text, i) => {
    const n = first + i;
    const on = hl && n >= hl.start_line && n <= hl.end_line;
    return h('div', { class: on ? 'cl cl-hl' : 'cl' },
      h('span', { class: 'cl-n', 'aria-hidden': 'true' }, n),
      h('span', { class: 'cl-t' }, text === '' ? '​' : text));
  });
  const range = hl ? (hl.start_line === hl.end_line ? `line ${hl.start_line}` : `lines ${hl.start_line}-${hl.end_line}`) : '';
  return h('figure', { class: 'excerpt' },
    h('figcaption', { class: 'excerpt-head' },
      label ? h('span', { class: 'pill' }, label) : null,
      h('span', { class: 'mono excerpt-path' }, ex.path),
      range ? h('span', { class: 'muted' }, range) : null),
    ex.current === false
      ? h('p', { class: 'warn-line', role: 'note' }, 'This file changed since indexing. Lines shown may not match the recorded ranges.')
      : null,
    h('div', { class: 'code', role: 'group', 'aria-label': `Source of ${ex.path}`, tabindex: '0' }, rows));
}

export function excerptList(excerpts, label, emptyText) {
  if (!excerpts || !excerpts.length) return h('p', { class: 'muted' }, emptyText);
  return h('div', { class: 'stack' }, excerpts.map((e) => excerptView(e, { label })));
}
