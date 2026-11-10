import { h } from '../dom.js';

// pager({total, offset, limit}, onOffset) -> nav element
export function pager(page, onOffset) {
  const { total, offset, limit } = page;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + limit, total);
  const prev = h('button', { type: 'button', class: 'btn', disabled: offset <= 0, onclick: () => onOffset(Math.max(0, offset - limit)) }, 'Previous');
  const next = h('button', { type: 'button', class: 'btn', disabled: offset + limit >= total, onclick: () => onOffset(offset + limit) }, 'Next');
  return h('nav', { class: 'pager', 'aria-label': 'Pagination' },
    prev, h('span', { class: 'pager-info', role: 'status' }, `${from}-${to} of ${total}`), next);
}
