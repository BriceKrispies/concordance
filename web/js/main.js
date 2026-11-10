// Bootstrap: load the status snapshot, open the event stream from its seq, start routing.
import * as events from './events.js';
import * as router from './router.js';
import { loadStatus, setState } from './store.js';
import { startStatusSync, reloadStatusNow } from './status-sync.js';
import { startActivity } from './activity-store.js';
import { initShell, markResynced, setActiveSection } from './shell.js';
import { h } from './dom.js';
import { errorBox } from './components/state.js';
import * as dashboard from './views/dashboard.js';
import * as findings from './views/findings.js';
import * as findingDetail from './views/finding-detail.js';
import * as specs from './views/specs.js';
import * as code from './views/code.js';
import * as graph from './views/graph.js';
import * as activity from './views/activity.js';

const main = document.getElementById('main');

function defineRoutes() {
  router.define('/', dashboard.mount, 'dashboard');
  router.define('/findings', findings.mount, 'findings');
  router.define('/findings/:id', findingDetail.mount, 'findings');
  router.define('/specs', specs.mountList, 'specs');
  router.define('/specs/:id', specs.mountDocument, 'specs');
  router.define('/claims/:id', specs.mountClaim, 'specs');
  router.define('/code', code.mountList, 'code');
  router.define('/code/:id', code.mountSymbol, 'code');
  router.define('/graph', graph.mountIndex, 'graph');
  router.define('/graph/:id', graph.mount, 'graph');
  router.define('/activity', activity.mount, 'activity');
  router.setNotFound((root) => {
    root.append(h('div', { class: 'view' }, h('h1', null, 'Not found'), h('p', null, 'No such page. '), h('a', { href: '#/' }, 'Back to the dashboard')));
  });
}

async function boot() {
  initShell();
  let status;
  try {
    status = await loadStatus();
  } catch (e) {
    main.append(errorBox(e, () => location.reload()));
    return;
  }
  defineRoutes();
  startStatusSync();
  startActivity();

  events.subscribe('resync', async () => {
    markResynced(setState);
    try { await reloadStatusNow(); } catch { /* shown by connection indicator */ }
    router.refreshCurrent();
  });

  events.start(status.seq, (conn) => setState({ conn }));
  router.start(main, setActiveSection);
}

boot();
