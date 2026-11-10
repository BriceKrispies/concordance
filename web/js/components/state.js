// Loading, empty and error states.
import { h } from '../dom.js';

export function loading(text = 'Loading') {
  return h('div', { class: 'state state-loading', role: 'status' }, h('span', { class: 'spinner', 'aria-hidden': 'true' }), text + '…');
}

export function empty(text, hint) {
  return h('div', { class: 'state state-empty' }, h('p', null, text), hint ? h('p', { class: 'muted' }, hint) : null);
}

export function errorBox(err, retry) {
  return h('div', { class: 'state state-error', role: 'alert' },
    h('p', null, h('strong', null, 'Something went wrong. '), err && err.message ? err.message : String(err)),
    retry ? h('button', { type: 'button', class: 'btn', onclick: retry }, 'Retry') : null);
}

export function inlineError(err) {
  return h('p', { class: 'inline-error', role: 'alert' }, err && err.message ? err.message : String(err));
}

// Page chrome shared by views: a heading row plus a body element to fill.
export function pageHeader(title, ...actions) {
  return h('div', { class: 'page-header' }, h('h1', null, title), actions.length ? h('div', { class: 'page-actions' }, actions) : null);
}
