#!/usr/bin/env bun

import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

export const DEFAULT_BASE_REF = 'main';
export const MAX_PULL_REQUESTS = 100;
const GITHUB_API = 'https://api.github.com';
const GITHUB_ACCEPT = 'application/vnd.github+json';
const GIT_TIMEOUT_MS = 30_000;

export type Disposition =
  | 'missing-secret'
  | 'malformed-secret'
  | 'draft'
  | 'fork'
  | 'closed'
  | 'non-main-base'
  | 'active-checks'
  | 'changed'
  | 'current'
  | 'conflict'
  | 'failed'
  | 'updated';

export type PullRequestSnapshot = {
  number: number;
  repository: string;
  state: 'open' | 'closed';
  draft: boolean;
  baseRef: string;
  baseSha: string;
  headRef: string;
  headSha: string;
  headRepository: string;
  mergeableState?: 'clean' | 'dirty' | 'unknown' | null;
};

export type Decision = {
  disposition: Disposition;
  reason: string;
};

export type CredentialState = 'missing' | 'malformed' | 'ready';

export type MergeOutcome =
  | { status: 'merged'; worktree: string; headSha: string }
  | { status: 'conflict' };

export type UpdatePorts = {
  readPullRequests: () => Promise<PullRequestSnapshot[]>;
  readPullRequest: (number: number) => Promise<PullRequestSnapshot>;
  hasActiveChecks: (snapshot: PullRequestSnapshot) => Promise<boolean>;
  fetchRefs: (snapshot: PullRequestSnapshot) => Promise<boolean>;
  isCurrent: (baseSha: string, headSha: string) => Promise<boolean>;
  mergeInWorktree: (snapshot: PullRequestSnapshot) => Promise<MergeOutcome>;
  pushWithLease: (snapshot: PullRequestSnapshot, worktree: string, mergedHeadSha: string) => Promise<'pushed' | 'lease-rejected'>;
  cleanupWorktree: (worktree: string) => Promise<void>;
};

export type UpdateSummary = {
  results: Array<{ number: number; decision: Decision }>;
  counts: Record<Disposition, number>;
};

const blankCounts = (): Record<Disposition, number> => ({
  'missing-secret': 0,
  'malformed-secret': 0,
  draft: 0,
  fork: 0,
  closed: 0,
  'non-main-base': 0,
  'active-checks': 0,
  changed: 0,
  current: 0,
  conflict: 0,
  failed: 0,
  updated: 0,
});

export const credentialState = (secret: string | undefined): CredentialState => {
  if (secret === undefined || secret === '') return 'missing';
  if (secret.trim() !== secret || secret.length < 20 || /[\r\n]/.test(secret)) return 'malformed';
  return 'ready';
};

const credentialDecision = (state: CredentialState): Decision | null => {
  if (state === 'missing') return { disposition: 'missing-secret', reason: 'dedicated update secret is absent' };
  if (state === 'malformed') return { disposition: 'malformed-secret', reason: 'dedicated update secret is malformed' };
  return null;
};

export const snapshotIdentity = (snapshot: PullRequestSnapshot): string => JSON.stringify({
  number: snapshot.number,
  repository: snapshot.repository,
  state: snapshot.state,
  draft: snapshot.draft,
  baseRef: snapshot.baseRef,
  baseSha: snapshot.baseSha,
  headRef: snapshot.headRef,
  headSha: snapshot.headSha,
  headRepository: snapshot.headRepository,
});

export const sameSnapshot = (left: PullRequestSnapshot, right: PullRequestSnapshot): boolean =>
  snapshotIdentity(left) === snapshotIdentity(right);

export const eligibilityDecision = (
  snapshot: PullRequestSnapshot,
  secret: CredentialState = 'ready',
  baseRef = DEFAULT_BASE_REF,
): Decision | null => {
  const credential = credentialDecision(secret);
  if (credential !== null) return credential;
  if (snapshot.state !== 'open') return { disposition: 'closed', reason: 'pull request is not open' };
  if (snapshot.draft) return { disposition: 'draft', reason: 'draft pull requests are never mutated' };
  if (snapshot.baseRef !== baseRef) return { disposition: 'non-main-base', reason: `base is not ${baseRef}` };
  if (snapshot.headRepository !== snapshot.repository) return { disposition: 'fork', reason: 'head is hosted in another repository' };
  if (snapshot.mergeableState === 'dirty') return { disposition: 'conflict', reason: 'GitHub reports a merge conflict' };
  return null;
};

export const raceDecision = (
  before: PullRequestSnapshot,
  after: PullRequestSnapshot,
): Decision | null => sameSnapshot(before, after)
  ? null
  : { disposition: 'changed', reason: 'base, head, state, draft, or repository identity changed' };

const failed = (reason: string): Decision => ({ disposition: 'failed', reason });

const samePostPushIdentity = (
  before: PullRequestSnapshot,
  after: PullRequestSnapshot,
  mergedHeadSha: string,
): boolean => before.number === after.number &&
  before.repository === after.repository &&
  after.state === 'open' &&
  !after.draft &&
  before.baseRef === after.baseRef &&
  before.baseSha === after.baseSha &&
  before.headRef === after.headRef &&
  before.headRepository === after.headRepository &&
  after.headSha === mergedHeadSha;

export const processPullRequest = async (
  listed: PullRequestSnapshot,
  ports: UpdatePorts,
  secret: CredentialState = 'ready',
): Promise<Decision> => {
  const listedGate = eligibilityDecision(listed, secret);
  if (listedGate !== null) return listedGate;

  let worktree: string | undefined;
  try {
    if (await ports.hasActiveChecks(listed)) {
      return { disposition: 'active-checks', reason: 'checks are still running' };
    }

    const initial = await ports.readPullRequest(listed.number);
    const initialRace = raceDecision(listed, initial);
    if (initialRace !== null) return initialRace;
    const initialGate = eligibilityDecision(initial, secret);
    if (initialGate !== null) return initialGate;
    if (await ports.hasActiveChecks(initial)) {
      return { disposition: 'active-checks', reason: 'checks became active during the identity recheck' };
    }

    if (!await ports.fetchRefs(initial)) {
      return { disposition: 'changed', reason: 'base or head changed while fetching immutable refs' };
    }
    if (await ports.isCurrent(initial.baseSha, initial.headSha)) {
      return { disposition: 'current', reason: 'head already contains the live base commit' };
    }

    const merge = await ports.mergeInWorktree(initial);
    if (merge.status === 'conflict') {
      return { disposition: 'conflict', reason: 'isolated merge could not be completed' };
    }
    worktree = merge.worktree;

    const beforePush = await ports.readPullRequest(initial.number);
    const beforePushRace = raceDecision(initial, beforePush);
    if (beforePushRace !== null) return beforePushRace;
    if (await ports.hasActiveChecks(beforePush)) {
      return { disposition: 'active-checks', reason: 'checks became active before the lease push' };
    }

    const push = await ports.pushWithLease(initial, worktree, merge.headSha);
    if (push === 'lease-rejected') {
      return { disposition: 'changed', reason: 'remote head changed before the lease push' };
    }

    const afterPush = await ports.readPullRequest(initial.number);
    if (!samePostPushIdentity(initial, afterPush, merge.headSha)) {
      return { disposition: 'changed', reason: 'base identity changed after the update' };
    }
    return { disposition: 'updated', reason: 'merged live main into the PR head and pushed with a safe lease' };
  } catch (error) {
    return failed(error instanceof Error ? error.message.split('\n', 1)[0] ?? 'operation failed' : 'operation failed');
  } finally {
    if (worktree !== undefined) {
      try {
        await ports.cleanupWorktree(worktree);
      } catch {
        // The update result is already determined; cleanup must not trigger another mutation.
      }
    }
  }
};

export const processAll = async (
  ports: UpdatePorts,
  secret: CredentialState = 'ready',
): Promise<UpdateSummary> => {
  const counts = blankCounts();
  const results: Array<{ number: number; decision: Decision }> = [];
  const configured = credentialDecision(secret);
  if (configured !== null) {
    counts[configured.disposition] += 1;
    return { results: [{ number: 0, decision: configured }], counts };
  }

  let pullRequests: PullRequestSnapshot[];
  try {
    pullRequests = await ports.readPullRequests();
  } catch {
    const decision = failed('unable to list pull requests');
    counts.failed += 1;
    return { results: [{ number: 0, decision }], counts };
  }

  for (const pullRequest of pullRequests.slice(0, MAX_PULL_REQUESTS)) {
    const decision = await processPullRequest(pullRequest, ports, secret);
    counts[decision.disposition] += 1;
    results.push({ number: pullRequest.number, decision });
  }
  return { results, counts };
};

const command = async (args: string[], cwd: string, token?: string): Promise<{ code: number; output: string }> => {
  const environment: Record<string, string> = {
    ...(process.env as Record<string, string>),
    GIT_TERMINAL_PROMPT: '0',
  };
  if (token !== undefined) {
    environment.GIT_CONFIG_COUNT = '1';
    environment.GIT_CONFIG_KEY_0 = 'http.https://github.com/.extraheader';
    environment.GIT_CONFIG_VALUE_0 = `AUTHORIZATION: bearer ${token}`;
  }
  const child = Bun.spawn({ cmd: ['git', ...args], cwd, env: environment, stdout: 'pipe', stderr: 'pipe' });
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    child.kill();
  }, GIT_TIMEOUT_MS);
  const [code, stdout, stderr] = await Promise.all([
    child.exited,
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ]);
  clearTimeout(timeout);
  return { code: timedOut ? 124 : code, output: timedOut ? 'git command timed out' : `${stdout}\n${stderr}`.trim() };
};

export class GitAdapter {
  public constructor(private readonly root: string, private readonly token: string) {}

  public async fetchRefs(snapshot: PullRequestSnapshot): Promise<boolean> {
    for (const ref of [snapshot.baseRef, snapshot.headRef]) {
      const checked = await command(['check-ref-format', '--branch', ref], this.root, this.token);
      if (checked.code !== 0) throw new Error('invalid branch ref');
    }
    const result = await command([
      'fetch', '--no-tags', 'origin',
      `+refs/heads/${snapshot.baseRef}:refs/remotes/origin/${snapshot.baseRef}`,
      `+refs/heads/${snapshot.headRef}:refs/remotes/origin/${snapshot.headRef}`,
    ], this.root, this.token);
    if (result.code !== 0) throw new Error('git fetch failed');
    const [base, head] = await Promise.all([
      command(['rev-parse', '--verify', `refs/remotes/origin/${snapshot.baseRef}^{commit}`], this.root, this.token),
      command(['rev-parse', '--verify', `refs/remotes/origin/${snapshot.headRef}^{commit}`], this.root, this.token),
    ]);
    if (base.code !== 0 || head.code !== 0) throw new Error('fetched ref resolution failed');
    return base.output === snapshot.baseSha && head.output === snapshot.headSha;
  }

  public async isCurrent(baseSha: string, headSha: string): Promise<boolean> {
    const result = await command(['merge-base', '--is-ancestor', baseSha, headSha], this.root, this.token);
    if (result.code === 0) return true;
    if (result.code === 1) return false;
    throw new Error('git ancestry check failed');
  }

  public async mergeInWorktree(snapshot: PullRequestSnapshot): Promise<MergeOutcome> {
    const temporaryRoot = await mkdtemp(join(tmpdir(), 'rusttable-pr-update-'));
    const worktree = join(temporaryRoot, 'head');
    const added = await command(['worktree', 'add', '--detach', worktree, snapshot.headSha], this.root, this.token);
    if (added.code !== 0) {
      await rm(temporaryRoot, { recursive: true, force: true });
      throw new Error('isolated worktree creation failed');
    }
    const merged = await command([
      '-c', 'user.name=RustTable PR Freshener',
      '-c', 'user.email=rusttable-pr-freshener@users.noreply.github.com',
      '-c', 'core.hooksPath=/dev/null',
      '-C', worktree, 'merge', '--no-edit', '--no-ff', snapshot.baseSha,
    ], this.root, this.token);
    if (merged.code !== 0) {
      await this.cleanupWorktree(worktree);
      return { status: 'conflict' };
    }
    const mergedHead = await command(['rev-parse', '--verify', 'HEAD^{commit}'], worktree, this.token);
    if (mergedHead.code !== 0 || !/^[0-9a-f]{40}$/.test(mergedHead.output)) {
      await this.cleanupWorktree(worktree);
      throw new Error('merged head resolution failed');
    }
    return { status: 'merged', worktree, headSha: mergedHead.output };
  }

  public async pushWithLease(snapshot: PullRequestSnapshot, worktree: string, mergedHeadSha: string): Promise<'pushed' | 'lease-rejected'> {
    const remote = await command(['ls-remote', 'origin', `refs/heads/${snapshot.headRef}`], this.root, this.token);
    const remoteHead = remote.output.split(/\s+/, 1)[0] ?? '';
    if (remote.code !== 0 || remoteHead !== snapshot.headSha) return 'lease-rejected';
    const pushed = await command([
      '-C', worktree, 'push', '--no-verify',
      `--force-with-lease=refs/heads/${snapshot.headRef}:${snapshot.headSha}`,
      'origin', `HEAD:refs/heads/${snapshot.headRef}`,
    ], this.root, this.token);
    if (pushed.code === 0) {
      const resolved = await command(['rev-parse', '--verify', 'HEAD^{commit}'], worktree, this.token);
      if (resolved.code !== 0 || resolved.output !== mergedHeadSha) throw new Error('pushed head resolution failed');
      return 'pushed';
    }
    if (/rejected|stale|lease/i.test(pushed.output)) return 'lease-rejected';
    throw new Error('git push failed');
  }

  public async cleanupWorktree(worktree: string): Promise<void> {
    await command(['worktree', 'remove', '--force', worktree], this.root, this.token);
    await rm(join(worktree, '..'), { recursive: true, force: true });
  }
}

type GitHubPullRequest = {
  number: number;
  state: string;
  draft: boolean | null;
  base: { ref: string; sha: string; repo: { full_name: string } | null };
  head: { ref: string; sha: string; repo: { full_name: string } | null };
  mergeable_state?: string | null;
};

const normalizePullRequest = (repository: string, value: GitHubPullRequest): PullRequestSnapshot => ({
  number: value.number,
  repository,
  state: value.state === 'open' ? 'open' : 'closed',
  draft: value.draft === true,
  baseRef: value.base?.ref ?? '',
  baseSha: value.base?.sha ?? '',
  headRef: value.head?.ref ?? '',
  headSha: value.head?.sha ?? '',
  headRepository: value.head?.repo?.full_name ?? '',
  mergeableState: value.mergeable_state === 'dirty' || value.mergeable_state === 'clean' || value.mergeable_state === 'unknown'
    ? value.mergeable_state
    : null,
});

export class GitHubAdapter {
  public constructor(private readonly repository: string, private readonly token: string) {}

  private async request(path: string): Promise<unknown> {
    const response = await fetch(`${GITHUB_API}${path}`, {
      headers: {
        Accept: GITHUB_ACCEPT,
        'X-GitHub-Api-Version': '2022-11-28',
        Authorization: `Bearer ${this.token}`,
      },
      signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok) throw new Error(`GitHub API request failed (${response.status})`);
    return response.json();
  }

  public async readPullRequests(): Promise<PullRequestSnapshot[]> {
    const values = await this.request(`/repos/${this.repository}/pulls?state=open&per_page=${MAX_PULL_REQUESTS}`) as GitHubPullRequest[];
    return values.map((value) => normalizePullRequest(this.repository, value));
  }

  public async readPullRequest(number: number): Promise<PullRequestSnapshot> {
    const value = await this.request(`/repos/${this.repository}/pulls/${number}`) as GitHubPullRequest;
    return normalizePullRequest(this.repository, value);
  }

  public async hasActiveChecks(snapshot: PullRequestSnapshot): Promise<boolean> {
    const checks = await this.request(`/repos/${this.repository}/commits/${snapshot.headSha}/check-runs?per_page=100`) as { check_runs?: Array<{ status?: string }> };
    if ((checks.check_runs ?? []).some((check) => check.status !== 'completed')) return true;
    const statuses = await this.request(`/repos/${this.repository}/commits/${snapshot.headSha}/status`) as { state?: string };
    return statuses.state === 'pending';
  }
}

const redact = (value: string, secret: string): string => secret === '' ? value : value.split(secret).join('[REDACTED]');

export const summaryLine = (summary: UpdateSummary): string => Object.entries(summary.counts)
  .filter(([, count]) => count > 0)
  .map(([disposition, count]) => `${disposition}=${count}`)
  .join(' ') || 'no eligible PRs';

export const run = async (): Promise<void> => {
  const secret = process.env.READY_PR_UPDATE_TOKEN;
  const credentials = credentialState(secret);
  const credential = credentialDecision(credentials);
  if (credential !== null) {
    console.log(`ready-pr-update: ${credential.disposition}; no branches changed`);
    return;
  }
  const repository = process.env.GITHUB_REPOSITORY ?? 'cgasgarth/RustTable';
  const root = process.env.GITHUB_WORKSPACE ?? process.cwd();
  try {
    const github = new GitHubAdapter(repository, secret ?? '');
    const git = new GitAdapter(root, secret ?? '');
    const summary = await processAll({
      readPullRequests: () => github.readPullRequests(),
      readPullRequest: (number) => github.readPullRequest(number),
      hasActiveChecks: (snapshot) => github.hasActiveChecks(snapshot),
      fetchRefs: (snapshot) => git.fetchRefs(snapshot),
      isCurrent: (baseSha, headSha) => git.isCurrent(baseSha, headSha),
      mergeInWorktree: (snapshot) => git.mergeInWorktree(snapshot),
      pushWithLease: (snapshot, worktree, mergedHeadSha) => git.pushWithLease(snapshot, worktree, mergedHeadSha),
      cleanupWorktree: (worktree) => git.cleanupWorktree(worktree),
    }, credentials);
    console.log(`ready-pr-update: ${summaryLine(summary)}`);
  } catch (error) {
    const message = redact(error instanceof Error ? error.message : 'unexpected updater failure', secret ?? '');
    console.log(`ready-pr-update: failed; ${message.split('\n', 1)[0] ?? 'unexpected updater failure'}`);
  }
};

if (import.meta.main) await run();
