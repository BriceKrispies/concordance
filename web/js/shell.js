// App shell: header (repo, connection, scan progress) and nav highlighting.
import { h, replaceChildren } from './dom.js';
import { basename, titleCase } from './format.js';
import { subscribe, getState } from './store.js';

let resyncTimer = null;

export function initShell() {
  const repo = document.getElementById('repo-name');
  const conn = document.getElementById('conn');
  const prog = document.getElementById('scan-progress');

  function paint(state) {
    const st = state.status;
    if (st && st.repository) {
      repo.textContent = basename(st.repository.root);
      repo.title = st.repository.root;
    }
    const label = state.resynced ? 'resynced' : state.conn;
    const text = { live: 'Live', reconnecting: 'Reconnecting', connecting: 'Connecting', resynced: 'Resynced' }[label] || label;
    replaceChildren(conn, h('span', { class: `conn-dot conn-${label}`, 'aria-hidden': 'true' }), h('span', null, text));
    conn.title = state.resynced ? 'The event stream fell behind, so data was reloaded.' : '';

    const scan = st && st.scan;
    prog.hidden = !scan;
    if (scan) {
      const total = scan.total || 0;
      const done = scan.done || 0;
      const bar = h('progress', { max: total || 1, value: total ? done : 0, 'aria-label': 'Scan progress' });
      replaceChildren(prog, bar, h('span', { class: 'scan-label' }, `Scanning: ${titleCase(scan.phase)}${total ? ` ${done}/${total}` : ''}`));
    }
  }

  subscribe(paint);
  paint(getState());

  document.getElementById('skip-link').addEventListener('click', () => {
    document.getElementById('main').focus();
  });
}

export function markResynced(setState) {
  setState({ resynced: true });
  clearTimeout(resyncTimer);
  resyncTimer = setTimeout(() => setState({ resynced: false }), 6000);
}

export function setActiveSection(section) {
  for (const a of document.querySelectorAll('.app-nav a')) {
    if (a.dataset.section === section) a.setAttribute('aria-current', 'page');
    else a.removeAttribute('aria-current');
  }
}
