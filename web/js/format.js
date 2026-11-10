// Formatting helpers and human-readable vocabularies.

export const STATUSES = ['violated', 'stale', 'ambiguous', 'unverified', 'verified', 'undocumented'];
export const SEVERITIES = ['critical', 'high', 'medium', 'low', 'info'];

export function titleCase(snake) {
  if (snake == null) return '';
  const t = String(snake).replace(/[_.]/g, ' ');
  return t.charAt(0).toUpperCase() + t.slice(1);
}

export const CATEGORY_TEXT = {
  structural_contradiction: 'Proven structural contradiction',
  suspected_semantic_contradiction: 'Suspected semantic contradiction (model-assessed, challenged)',
  missing_evidence: 'Missing evidence: nothing yet confirms or refutes this claim',
  unclear_documentation: 'Unclear documentation: the specification is ambiguous or inconsistent',
  missing_documentation: 'Missing documentation: public code with no documented contract',
  invalidated_evidence: 'Invalidated evidence: earlier evidence no longer applies after a change',
  supported: 'Supported by evidence',
  test_failure: 'Test failure: a test that verifies this requirement failed',
};

export const ACTION_TEXT = {
  change_implementation: 'Change the implementation',
  update_descriptive_documentation: 'Update the descriptive documentation',
  clarify_specification: 'Clarify the specification',
  establish_specification: 'Establish a specification',
  gather_stronger_evidence: 'Gather stronger evidence',
  accept_behavior_change: 'Accept the behavior change',
  no_action_needed: 'No action needed',
};

export const STATUS_HELP = {
  violated: 'Code contradicts the claim',
  stale: 'Evidence is out of date',
  ambiguous: 'Claim or mapping is unclear',
  unverified: 'No evidence either way',
  verified: 'Evidence supports the claim',
  undocumented: 'Public code without a documented contract',
};

export function rangeStr(r) {
  if (!r) return '';
  return r.start_line === r.end_line ? `${r.path}:${r.start_line}` : `${r.path}:${r.start_line}-${r.end_line}`;
}

export function lineSpan(r) {
  if (!r) return '';
  return r.start_line === r.end_line ? `line ${r.start_line}` : `lines ${r.start_line}-${r.end_line}`;
}

export function pct(x) {
  return typeof x === 'number' ? `${Math.round(x * 100)}%` : '';
}

// Timestamps are Unix milliseconds.
export function when(ms) {
  if (typeof ms !== 'number') return '';
  return new Date(ms).toLocaleString();
}

export function ago(ms) {
  if (typeof ms !== 'number') return '';
  const d = Math.max(0, Date.now() - ms) / 1000;
  if (d < 5) return 'just now';
  if (d < 60) return `${Math.floor(d)}s ago`;
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
  return `${Math.floor(d / 86400)}d ago`;
}

export function duration(ms) {
  if (typeof ms !== 'number') return '';
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.floor(ms / 60000)} m ${Math.round((ms % 60000) / 1000)} s`;
}

export function basename(p) {
  const parts = String(p || '').replace(/\\/g, '/').split('/').filter(Boolean);
  return parts.length ? parts[parts.length - 1] : '';
}

export function plural(n, one, many) {
  return `${n} ${n === 1 ? one : many || one + 's'}`;
}

// Resolution is an object {kind, ...} in core; tolerate a bare string.
export function resolutionKind(res) {
  if (!res) return 'unresolved';
  return typeof res === 'string' ? res : res.kind || 'unresolved';
}

export function resolutionText(res) {
  const k = resolutionKind(res);
  return k === 'ambiguous' && res && res.candidates ? `ambiguous (${res.candidates} candidates)` : k.replace(/_/g, ' ');
}

export function isWeakResolution(res) {
  return ['lexical', 'ambiguous', 'unresolved'].includes(resolutionKind(res));
}

export function methodText(m) {
  if (!m) return '';
  if (m.kind === 'structural') return `structural: ${String(m.check).replace(/_/g, ' ')}`;
  if (m.kind === 'semantic') return `semantic model${m.model ? ' ' + m.model : ''}`;
  if (m.kind === 'adversarial') return `adversarial challenge${m.model ? ' ' + m.model : ''}`;
  if (m.kind === 'test') return `test ${m.test || ''}: ${String(m.outcome || '').replace(/_/g, ' ')}`;
  return String(m.kind || '');
}

export function referenceTargetText(t) {
  if (!t) return '';
  switch (t.kind) {
    case 'symbol': return (t.path || []).join('::');
    case 'file': return t.line ? `${t.path}:${t.line}` : t.path;
    case 'section': return `${t.document}${t.anchor ? '#' + t.anchor : ''}`;
    case 'outside_repo': return t.text;
    case 'external': return t.url; // shown as text only, never as a link
    default: return '';
  }
}
