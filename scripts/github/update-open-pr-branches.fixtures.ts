import type { PullRequestSnapshot } from './update-open-pr-branches';

export const READY_SECRET = 'github_pat_' + 'x'.repeat(40);

export const snapshot = (overrides: Partial<PullRequestSnapshot> = {}): PullRequestSnapshot => ({
  number: 362,
  repository: 'cgasgarth/RustTable',
  state: 'open',
  draft: false,
  baseRef: 'main',
  baseSha: '1'.repeat(40),
  headRef: 'feature/photo-grid',
  headSha: '2'.repeat(40),
  headRepository: 'cgasgarth/RustTable',
  mergeableState: null,
  ...overrides,
});

export const dispositions = {
  missingSecret: snapshot(),
  malformedSecret: snapshot(),
  draft: snapshot({ draft: true }),
  fork: snapshot({ headRepository: 'someone/RustTable' }),
  closed: snapshot({ state: 'closed' }),
  nonMainBase: snapshot({ baseRef: 'release' }),
  activeChecks: snapshot(),
  changed: snapshot({ headSha: '3'.repeat(40) }),
  current: snapshot(),
  conflict: snapshot({ mergeableState: 'dirty' }),
  failed: snapshot(),
  updated: snapshot(),
} as const;
