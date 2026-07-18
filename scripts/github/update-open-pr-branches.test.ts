import { describe, expect, test } from 'bun:test';
import {
  credentialState,
  eligibilityDecision,
  processAll,
  processPullRequest,
  raceDecision,
  summaryLine,
  type MergeOutcome,
  type PullRequestSnapshot,
  type UpdatePorts,
} from './update-open-pr-branches';
import { dispositions, READY_SECRET, snapshot } from './update-open-pr-branches.fixtures';

const basePorts = (overrides: Partial<UpdatePorts> = {}): UpdatePorts => ({
  readPullRequests: async () => [snapshot()],
  readPullRequest: async () => snapshot(),
  hasActiveChecks: async () => false,
  fetchRefs: async () => true,
  isCurrent: async () => false,
  mergeInWorktree: async (): Promise<MergeOutcome> => ({
    status: 'merged',
    worktree: '/tmp/freshener-worktree',
    headSha: '2'.repeat(40),
  }),
  pushWithLease: async () => 'pushed',
  cleanupWorktree: async () => undefined,
  ...overrides,
});

describe('ready PR freshness credentials and eligibility', () => {
  test.each([
    ['missing secret', undefined, 'missing'],
    ['malformed secret', 'too-short', 'malformed'],
    ['least privilege secret', READY_SECRET, 'ready'],
  ])('%s is bounded', (_name, secret, expected) => {
    expect(credentialState(secret)).toBe(expected);
  });

  test.each([
    ['draft', dispositions.draft, 'draft'],
    ['fork', dispositions.fork, 'fork'],
    ['closed', dispositions.closed, 'closed'],
    ['non-main base', dispositions.nonMainBase, 'non-main-base'],
    ['conflict', dispositions.conflict, 'conflict'],
  ])('never mutates %s', (_name, value, expected) => {
    expect(eligibilityDecision(value, 'ready')?.disposition).toBe(expected);
  });
});

describe('ready PR freshness race-safe processing', () => {
  test('rechecks base and head identities before any merge', async () => {
    let merged = false;
    const result = await processPullRequest(snapshot(), basePorts({
      readPullRequest: async () => dispositions.changed,
      mergeInWorktree: async () => {
        merged = true;
        return { status: 'conflict' };
      },
    }), 'ready');
    expect(result.disposition).toBe('changed');
    expect(merged).toBe(false);
  });

  test('fails closed when fetched refs no longer match the API snapshot', async () => {
    let merged = false;
    const result = await processPullRequest(snapshot(), basePorts({
      fetchRefs: async () => false,
      mergeInWorktree: async () => {
        merged = true;
        return { status: 'conflict' };
      },
    }), 'ready');
    expect(result.disposition).toBe('changed');
    expect(merged).toBe(false);
  });

  test('skips active checks before mutation and rechecks them before push', async () => {
    let calls = 0;
    let pushed = false;
    const result = await processPullRequest(snapshot(), basePorts({
      hasActiveChecks: async () => {
        calls += 1;
        return calls > 1;
      },
      pushWithLease: async () => {
        pushed = true;
        return 'pushed';
      },
    }), 'ready');
    expect(result.disposition).toBe('active-checks');
    expect(pushed).toBe(false);
  });

  test('reports current without creating a worktree', async () => {
    let created = false;
    const result = await processPullRequest(snapshot(), basePorts({
      isCurrent: async () => true,
      mergeInWorktree: async () => {
        created = true;
        return { status: 'conflict' };
      },
    }), 'ready');
    expect(result.disposition).toBe('current');
    expect(created).toBe(false);
  });

  test('reports isolated merge conflicts without pushing', async () => {
    let pushed = false;
    const result = await processPullRequest(snapshot(), basePorts({
      mergeInWorktree: async () => ({ status: 'conflict' }),
      pushWithLease: async () => {
        pushed = true;
        return 'pushed';
      },
    }), 'ready');
    expect(result.disposition).toBe('conflict');
    expect(pushed).toBe(false);
  });

  test('turns a rejected lease into a changed no-op', async () => {
    const result = await processPullRequest(snapshot(), basePorts({
      pushWithLease: async () => 'lease-rejected',
    }), 'ready');
    expect(result.disposition).toBe('changed');
  });

  test('updates once and cleanup is isolated from the result', async () => {
    let pushed = 0;
    let cleaned = 0;
    const result = await processPullRequest(snapshot(), basePorts({
      pushWithLease: async () => {
        pushed += 1;
        return 'pushed';
      },
      cleanupWorktree: async () => {
        cleaned += 1;
        throw new Error('cleanup is best effort');
      },
    }), 'ready');
    expect(result.disposition).toBe('updated');
    expect(pushed).toBe(1);
    expect(cleaned).toBe(1);
  });

  test.each([
    ['closed', snapshot({ state: 'closed' })],
    ['draft', snapshot({ draft: true })],
    ['base ref', snapshot({ baseRef: 'release' })],
    ['base sha', snapshot({ baseSha: '3'.repeat(40) })],
    ['head ref', snapshot({ headRef: 'other-branch' })],
    ['head repository', snapshot({ headRepository: 'other/repo' })],
    ['repository', snapshot({ repository: 'other/repo' })],
  ] as const)('reports a post-push %s identity race without claiming success', async (_name, after) => {
    let reads = 0;
    const result = await processPullRequest(snapshot(), basePorts({
      readPullRequest: async () => {
        reads += 1;
        return reads === 1 ? snapshot() : reads === 2 ? snapshot() : after;
      },
    }), 'ready');
    expect(result.disposition).toBe('changed');
  });

  test('a second run that is already current is idempotent', async () => {
    let pushCalls = 0;
    const ports = basePorts({
      isCurrent: async (_baseSha, headSha) => headSha === '4'.repeat(40),
      readPullRequest: async () => snapshot({ headSha: '4'.repeat(40) }),
      pushWithLease: async () => {
        pushCalls += 1;
        return 'pushed';
      },
    });
    const result = await processPullRequest(snapshot({ headSha: '4'.repeat(40) }), ports, 'ready');
    expect(result.disposition).toBe('current');
    expect(pushCalls).toBe(0);
  });
});

describe('ready PR freshness bounded summaries', () => {
  test('missing secret performs no listing and returns a bounded no-op', async () => {
    let listed = false;
    const summary = await processAll(basePorts({
      readPullRequests: async () => {
        listed = true;
        return [];
      },
    }), 'missing');
    expect(listed).toBe(false);
    expect(summary.counts['missing-secret']).toBe(1);
    expect(summaryLine(summary)).toBe('missing-secret=1');
  });

  test('one failed PR does not prevent the next PR', async () => {
    const values: PullRequestSnapshot[] = [snapshot({ number: 1 }), snapshot({ number: 2 })];
    const summary = await processAll(basePorts({
      readPullRequests: async () => values,
      readPullRequest: async (number) => {
        if (number === 1) throw new Error('API failure');
        return snapshot({ number });
      },
    }), 'ready');
    expect(summary.results.map(({ decision }) => decision.disposition)).toEqual(['failed', 'updated']);
  });
});

test('snapshot race compares immutable identities including repository and draft state', () => {
  expect(raceDecision(snapshot(), snapshot())).toBeNull();
  expect(raceDecision(snapshot(), snapshot({ draft: true }))?.disposition).toBe('changed');
  expect(raceDecision(snapshot(), snapshot({ headRepository: 'other/repo' }))?.disposition).toBe('changed');
});
