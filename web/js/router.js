// Hash router. A view is mount(root, ctx) -> { refresh?, destroy? } | undefined.
import { subscribe as subscribeEvents } from './events.js';
import { throttle } from './dom.js';

const table = []; // { segments, mount, section }
let rootEl = null;
let current = null; // { view, ctx, abort, cleanups }
let notFound = null;
let onNavigate = () => {};

export function define(pattern, mount, section) {
  table.push({ segments: pattern.split('/').filter(Boolean), mount, section });
}

export function setNotFound(mount) {
  notFound = mount;
}

export function parseHash(hash = location.hash) {
  let raw = hash.startsWith('#') ? hash.slice(1) : hash;
  if (!raw.startsWith('/')) raw = '/' + raw;
  const [pathPart, queryPart = ''] = raw.split('?');
  const segments = pathPart.split('/').filter(Boolean).map((s) => {
    try { return decodeURIComponent(s); } catch { return s; }
  });
  return { segments, query: Object.fromEntries(new URLSearchParams(queryPart)) };
}

function match(segments) {
  for (const r of table) {
    if (r.segments.length !== segments.length) continue;
    const params = {};
    let ok = true;
    r.segments.forEach((p, i) => {
      if (p.startsWith(':')) params[p.slice(1)] = segments[i];
      else if (p !== segments[i]) ok = false;
    });
    if (ok) return { route: r, params };
  }
  return null;
}

function eventMatches(patterns, type) {
  return patterns.some((p) => (p.endsWith('*') ? type.startsWith(p.slice(0, -1)) : p === type));
}

function makeCtx(params, query) {
  const abort = new AbortController();
  const cleanups = [];
  const ctx = {
    params,
    query,
    signal: abort.signal,
    cleanup: (fn) => cleanups.push(fn),
    // Run fn at most once per `ms` after any event whose type matches (e.g. 'finding.*').
    live(patterns, fn, ms = 2000) {
      const t = throttle(fn, ms);
      const unsub = subscribeEvents('*', (env) => { if (eventMatches(patterns, env.type)) t(); });
      cleanups.push(unsub, () => t.cancel());
    },
    // Re-run fn when a stream resync happens.
    onResync(fn) {
      cleanups.push(subscribeEvents('resync', fn));
    },
  };
  return { ctx, abort, cleanups };
}

function teardown() {
  if (!current) return;
  current.abort.abort();
  for (const fn of current.cleanups) { try { fn(); } catch (e) { console.error(e); } }
  try { if (current.view && current.view.destroy) current.view.destroy(); } catch (e) { console.error(e); }
  current = null;
}

export function render() {
  teardown();
  const { segments, query } = parseHash();
  const m = match(segments);
  const mount = m ? m.route.mount : notFound;
  const section = m ? m.route.section : null;
  const { ctx, abort, cleanups } = makeCtx(m ? m.params : {}, query);
  rootEl.replaceChildren();
  current = { view: null, ctx, abort, cleanups };
  current.view = mount(rootEl, ctx) || null;
  onNavigate(section);
  window.scrollTo(0, 0);
}

export function refreshCurrent() {
  if (current && current.view && current.view.refresh) current.view.refresh();
}

// Update the hash without triggering a re-render (view keeps its own state).
export function replaceHash(hash) {
  history.replaceState(null, '', hash);
}

export function start(root, navigateCallback) {
  rootEl = root;
  onNavigate = navigateCallback || onNavigate;
  window.addEventListener('hashchange', render);
  render();
}
