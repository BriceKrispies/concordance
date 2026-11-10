import { h, replaceChildren, throttle } from '../dom.js';
import { getItems, subscribe, fillActivity } from '../activity-store.js';
import { describe, categoryOf, CATEGORIES } from '../activity-format.js';
import { isAbort } from '../api.js';
import { when } from '../format.js';
import { loading, empty, errorBox, pageHeader } from '../components/state.js';

export function mount(root, ctx) {
  const enabled = new Set(CATEGORIES.map(([k]) => k));
  const list = h('div', null);
  const chips = h('div', { class: 'chips', role: 'group', 'aria-label': 'Filter by category' });
  let ready = false;

  function paintChips() {
    replaceChildren(chips, CATEGORIES.map(([key, label]) => h('button', {
      type: 'button', class: `chip-btn${enabled.has(key) ? ' on' : ''}`, 'aria-pressed': String(enabled.has(key)),
      onclick: () => { if (enabled.has(key)) enabled.delete(key); else enabled.add(key); paintChips(); paint(); },
    }, label)));
  }

  function paint() {
    if (!ready) return;
    const items = getItems().filter((e) => enabled.has(categoryOf(e.type)));
    if (!items.length) {
      replaceChildren(list, empty('No activity to show.', 'Events appear here as files change and scans run.'));
      return;
    }
    replaceChildren(list, h('ol', { class: 'feed plain', 'aria-label': 'Event feed, newest first' }, items.map((env) => {
      const { level, body } = describe(env);
      return h('li', { class: `feed-item feed-${level}` },
        h('time', { class: 'feed-time mono small', title: when(env.at) }, env.at ? new Date(env.at).toLocaleTimeString() : ''),
        h('span', { class: 'feed-seq muted small mono' }, `#${env.seq}`),
        level === 'warn' || level === 'error' ? h('span', { class: `sev-tag sev-tag-${level}` }, level === 'warn' ? 'Warning' : 'Error') : null,
        h('span', { class: 'feed-body' }, body));
    })));
  }
  const paintSoon = throttle(paint, 250);

  root.append(h('div', { class: 'view' }, pageHeader('Activity'), chips, list));
  paintChips();
  replaceChildren(list, loading('Loading recent events'));

  async function init() {
    try {
      await fillActivity(ctx.signal);
      ready = true;
      paint();
    } catch (e) {
      if (isAbort(e)) return;
      replaceChildren(list, errorBox(e, () => { replaceChildren(list, loading()); init(); }));
    }
  }
  init();
  ctx.cleanup(subscribe(paintSoon));
  ctx.cleanup(() => paintSoon.cancel());
  ctx.onResync(() => { fillActivity(ctx.signal, true).then(paint, () => {}); });
  return { refresh: paint };
}
