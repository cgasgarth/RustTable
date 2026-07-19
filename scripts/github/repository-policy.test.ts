import { describe, expect, test } from 'bun:test';
import {
  applyPolicy,
  expectedProtectionPolicy,
  expectedRepositoryPolicy,
  findPolicyDrifts,
  normalizeProtection,
  policyState,
  redactOutput,
  readPolicy,
  run,
  type Request,
  type Transport,
} from './repository-policy';

const repositoryFixture = (): Record<string, unknown> => ({
  default_branch: 'main',
  allow_squash_merge: true,
  allow_merge_commit: false,
  allow_rebase_merge: false,
  irrelevant_field: 'ignored',
});

const protectionFixture = (): Record<string, unknown> => ({
  required_status_checks: { strict: true, checks: [] },
  enforce_admins: { enabled: true },
  required_pull_request_reviews: {
    required_approving_review_count: 0,
    bypass_pull_request_allowances: { users: [], teams: [], apps: [] },
  },
  required_linear_history: true,
  allow_force_pushes: false,
  allow_deletions: false,
  required_conversation_resolution: { enabled: true },
  restrictions: null,
});

const compliantState = () => policyState(repositoryFixture(), protectionFixture());

const fakeTransport = (repository = repositoryFixture(), protection: unknown = protectionFixture()): {
  calls: Array<{ path: string; request: Request }>;
  transport: Transport;
} => {
  const calls: Array<{ path: string; request: Request }> = [];
  const transport: Transport = async (path, request = {}) => {
    calls.push({ path, request });
    if (path.endsWith('/protection')) return protection;
    return repository;
  };
  return { calls, transport };
};

describe('repository policy normalization', () => {
  test('accepts the compliant GitHub response and ignores irrelevant fields', () => {
    expect(compliantState().repository).toEqual(expectedRepositoryPolicy);
    expect(compliantState().protection).toEqual(expectedProtectionPolicy);
    expect(findPolicyDrifts(compliantState())).toEqual([]);
  });

  test('reports an unprotected branch as a specific fail-closed drift', () => {
    const drifts = findPolicyDrifts(policyState(repositoryFixture(), null));
    expect(drifts.map(({ field }) => field)).toEqual(['main.protection']);
  });

  test.each([
    ['repository default branch', (value: Record<string, unknown>) => { value.default_branch = 'develop'; }, 'repository.default_branch'],
    ['squash merges', (value: Record<string, unknown>) => { value.allow_squash_merge = false; }, 'repository.allow_squash_merge'],
    ['merge commits', (value: Record<string, unknown>) => { value.allow_merge_commit = true; }, 'repository.allow_merge_commit'],
    ['rebase merges', (value: Record<string, unknown>) => { value.allow_rebase_merge = true; }, 'repository.allow_rebase_merge'],
  ])('reports %s drift', (_name, mutate, field) => {
    const repository = repositoryFixture();
    mutate(repository);
    expect(findPolicyDrifts(policyState(repository, protectionFixture())).map(({ field: actual }) => actual)).toContain(field);
  });

  test.each([
    ['status context', (value: Record<string, unknown>) => { value.required_status_checks = { strict: true, checks: [{ context: 'wrong' }] }; }],
    ['non-strict status checks', (value: Record<string, unknown>) => { value.required_status_checks = { strict: false, checks: [] }; }],
    ['administrator enforcement', (value: Record<string, unknown>) => { value.enforce_admins = { enabled: false }; }],
    ['conversation resolution', (value: Record<string, unknown>) => { value.required_conversation_resolution = { enabled: false }; }],
    ['linear history', (value: Record<string, unknown>) => { value.required_linear_history = false; }],
    ['force pushes', (value: Record<string, unknown>) => { value.allow_force_pushes = true; }],
    ['branch deletion', (value: Record<string, unknown>) => { value.allow_deletions = true; }],
    ['actor bypass', (value: Record<string, unknown>) => { value.required_pull_request_reviews = { required_approving_review_count: 0, bypass_pull_request_allowances: { users: ['owner'], teams: [], apps: [] } }; }],
    ['team bypass', (value: Record<string, unknown>) => { value.required_pull_request_reviews = { required_approving_review_count: 0, bypass_pull_request_allowances: { users: [], teams: ['maintainers'], apps: [] } }; }],
    ['app bypass', (value: Record<string, unknown>) => { value.required_pull_request_reviews = { required_approving_review_count: 0, bypass_pull_request_allowances: { users: [], teams: [], apps: ['automation'] } }; }],
  ])('reports %s drift', (_name, mutate) => {
    const protection = protectionFixture();
    mutate(protection);
    expect(findPolicyDrifts(policyState(repositoryFixture(), protection)).length).toBeGreaterThan(0);
  });

  test('reports simultaneous drifts together', () => {
    const repository = repositoryFixture();
    repository.allow_merge_commit = true;
    const protection = protectionFixture();
    protection.allow_force_pushes = true;
    protection.required_status_checks = { strict: false, checks: [{ context: 'wrong' }] };
    expect(findPolicyDrifts(policyState(repository, protection)).map(({ field }) => field)).toEqual([
      'repository.allow_merge_commit',
      'main.required_status_checks',
      'main.allow_force_pushes',
    ]);
  });
});

describe('repository policy transport and apply', () => {
  test('read treats only a branch-protection 404 as unprotected', async () => {
    const { calls, transport: base } = fakeTransport();
    const transport: Transport = async (path, request) => {
      if (path.endsWith('/protection')) throw new Error('404 Branch not protected');
      return base(path, request);
    };
    expect((await readPolicy('cgasgarth/RustTable', 'main', transport)).protection).toBeNull();
    expect(calls).toHaveLength(1);
  });

  test('confirmation mismatch makes zero write requests', async () => {
    const { calls, transport } = fakeTransport();
    await expect(run(['--apply', 'cgasgarth/Other', 'main'], transport)).rejects.toThrow('usage');
    expect(calls).toEqual([]);
  });

  test('apply updates only the declared settings and verifies the result', async () => {
    const calls: Array<{ path: string; request: Request }> = [];
    let repository: unknown = repositoryFixture();
    let protection: unknown = protectionFixture();
    const transport: Transport = async (path, request = {}) => {
      calls.push({ path, request });
      if (request.method === 'PATCH') repository = repositoryFixture();
      if (request.method === 'PUT') protection = protectionFixture();
      if (path.endsWith('/protection')) return protection;
      return repository;
    };
    await applyPolicy('cgasgarth/RustTable', 'main', transport);
    expect(calls.map(({ path, request }) => `${request.method ?? 'GET'} ${path}`)).toEqual([
      'PATCH /repos/cgasgarth/RustTable',
      'PUT /repos/cgasgarth/RustTable/branches/main/protection',
      'GET /repos/cgasgarth/RustTable',
      'GET /repos/cgasgarth/RustTable/branches/main/protection',
    ]);
    expect(JSON.parse(calls[0]?.request.body ?? '{}')).toEqual({
      default_branch: 'main', allow_squash_merge: true, allow_merge_commit: false, allow_rebase_merge: false,
    });
    const protectionBody = JSON.parse(calls[1]?.request.body ?? '{}') as Record<string, any>;
    expect(protectionBody.required_status_checks).toEqual({ strict: true, contexts: [] });
    expect(protectionBody.required_pull_request_reviews.bypass_pull_request_allowances).toBeUndefined();
  });

  test('failed update is returned without verification', async () => {
    let calls = 0;
    const transport: Transport = async () => {
      calls += 1;
      throw new Error('500 update failed');
    };
    await expect(applyPolicy('cgasgarth/RustTable', 'main', transport)).rejects.toThrow('500 update failed');
    expect(calls).toBe(1);
  });

  test('failed post-update verification is fatal', async () => {
    const transport: Transport = async (path, request = {}) => {
      if (request.method === 'PUT') return protectionFixture();
      if (request.method === 'PATCH') return repositoryFixture();
      if (path.endsWith('/protection')) return { ...protectionFixture(), allow_deletions: true };
      return repositoryFixture();
    };
    await expect(applyPolicy('cgasgarth/RustTable', 'main', transport)).rejects.toThrow('post-apply verification failed');
  });

  test('redacts authentication material from diagnostics', () => {
    expect(redactOutput('authorization=secret-token body=secret-token', 'secret-token')).toBe(
      'authorization=[REDACTED] body=[REDACTED]',
    );
  });
});
