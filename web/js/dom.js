// Safe DOM construction. Never uses innerHTML or friends: all text goes through
// text nodes, and href values must be hash routes built by this app.

const SVG_NS = 'http://www.w3.org/2000/svg';

function safeHref(v) {
  return typeof v === 'string' && v.startsWith('#/');
}

function applyAttrs(el, attrs) {
  if (!attrs) return;
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.setAttribute('class', v);
    else if (k === 'dataset') Object.assign(el.dataset, v);
    else if (k.startsWith('on')) {
      if (typeof v === 'function') el.addEventListener(k.slice(2), v);
    } else if (k === 'href') {
      if (safeHref(v)) el.setAttribute('href', v);
    } else if (k === 'src' || k === 'srcdoc' || k === 'style') continue;
    else if (k === 'value' || k === 'checked' || k === 'selected' || k === 'disabled') {
      el[k] = v;
      if (k !== 'value' && v) el.setAttribute(k, '');
    } else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, String(v));
  }
}

function append(el, children) {
  for (const c of children) {
    if (c == null || c === false || c === true) continue;
    if (Array.isArray(c)) append(el, c);
    else if (c instanceof Node) el.appendChild(c);
    else el.appendChild(document.createTextNode(String(c)));
  }
}

export function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  applyAttrs(el, attrs);
  append(el, children);
  return el;
}

export function s(tag, attrs, ...children) {
  const el = document.createElementNS(SVG_NS, tag);
  applyAttrs(el, attrs);
  append(el, children);
  return el;
}

export function clear(el) {
  while (el.firstChild) el.removeChild(el.firstChild);
  return el;
}

export function replaceChildren(el, ...children) {
  clear(el);
  append(el, children);
  return el;
}

export function throttle(fn, ms) {
  let last = 0;
  let timer = null;
  const run = () => { timer = null; last = Date.now(); fn(); };
  const t = () => {
    if (timer) return;
    const wait = last + ms - Date.now();
    if (wait <= 0) run(); else timer = setTimeout(run, wait);
  };
  t.cancel = () => { if (timer) clearTimeout(timer); timer = null; };
  return t;
}

export function debounce(fn, ms) {
  let timer = null;
  const d = (...a) => { clearTimeout(timer); timer = setTimeout(() => fn(...a), ms); };
  d.cancel = () => clearTimeout(timer);
  return d;
}

// Hash-route builder. Path segments and query values are percent-encoded.
export function route(path, query) {
  const q = new URLSearchParams();
  if (query) for (const [k, v] of Object.entries(query)) if (v !== '' && v != null) q.set(k, v);
  const qs = q.toString();
  return '#/' + path.split('/').filter(Boolean).map(encodeURIComponent).join('/') + (qs ? '?' + qs : '');
}

// Link target for any node id, chosen by id prefix.
export function nodeHref(id) {
  if (typeof id !== 'string') return null;
  if (id.startsWith('fnd_')) return route('findings/' + id);
  if (id.startsWith('doc_')) return route('specs/' + id);
  if (id.startsWith('clm_')) return route('claims/' + id);
  if (id.startsWith('sym_')) return route('code/' + id);
  if (/^(sec_|src_)/.test(id)) return route('graph/' + id);
  return null;
}

export function nodeLink(id, label, cls) {
  const href = nodeHref(id);
  return href ? h('a', { href, class: cls || 'mono' }, label || id) : h('span', { class: cls || 'mono' }, label || id);
}
