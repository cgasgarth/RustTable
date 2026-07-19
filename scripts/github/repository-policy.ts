#!/usr/bin/env bun

const API_ROOT = 'https://api.github.com';
const ACCEPT = 'application/vnd.github+json';
const APPLY_FLAG = '--apply';

export type RepositoryPolicy = {
  default_branch: string;
  allow_squash_merge: boolean;
  allow_merge_commit: boolean;
  allow_rebase_merge: boolean;
};

export type ProtectionPolicy = {
  required_status_checks: {
    strict: boolean;
    contexts: string[];
  };
  required_pull_request_reviews: {
    required_approving_review_count: number;
    bypass_pull_request_allowances: {
      users: string[];
      teams: string[];
      apps: string[];
    };
  };
  enforce_admins: boolean;
  required_conversation_resolution: boolean;
  required_linear_history: boolean;
  allow_force_pushes: boolean;
  allow_deletions: boolean;
};

export type PolicyState = {
  repository: RepositoryPolicy;
  protection: ProtectionPolicy | null;
};

export type PolicyDrift = {
  field: string;
  expected: unknown;
  observed: unknown;
};

export type Request = {
  method?: string;
  body?: string;
  headers?: Record<string, string>;
};

export type Transport = (path: string, request?: Request) => Promise<unknown>;

export const expectedRepositoryPolicy: RepositoryPolicy = {
  default_branch: 'main',
  allow_squash_merge: true,
  allow_merge_commit: false,
  allow_rebase_merge: false,
};

export const expectedProtectionPolicy: ProtectionPolicy = {
  required_status_checks: { strict: true, contexts: [] },
  required_pull_request_reviews: {
    required_approving_review_count: 0,
    bypass_pull_request_allowances: { users: [], teams: [], apps: [] },
  },
  enforce_admins: true,
  required_conversation_resolution: true,
  required_linear_history: true,
  allow_force_pushes: false,
  allow_deletions: false,
};

const asRecord = (value: unknown): Record<string, unknown> =>
  typeof value === 'object' && value !== null ? value as Record<string, unknown> : {};

const asBoolean = (value: unknown): boolean => value === true;

const asString = (value: unknown): string => typeof value === 'string' ? value : '';

const asNumber = (value: unknown): number => typeof value === 'number' ? value : -1;

const names = (value: unknown): string[] => Array.isArray(value)
  ? value.map((entry) => typeof entry === 'string' ? entry : asString(asRecord(entry).login ?? asRecord(entry).slug ?? asRecord(entry).name)).filter(Boolean).sort()
  : [];

const contexts = (value: unknown): string[] => {
  if (!Array.isArray(value)) return [];
  return value.map((entry) => {
    if (typeof entry === 'string') return entry;
    const record = asRecord(entry);
    return asString(record.context ?? record.name);
  }).filter(Boolean).sort();
};

export const normalizeRepository = (value: unknown): RepositoryPolicy => {
  const record = asRecord(value);
  return {
    default_branch: asString(record.default_branch),
    allow_squash_merge: asBoolean(record.allow_squash_merge),
    allow_merge_commit: asBoolean(record.allow_merge_commit),
    allow_rebase_merge: asBoolean(record.allow_rebase_merge),
  };
};

export const normalizeProtection = (value: unknown): ProtectionPolicy | null => {
  if (value === null) return null;
  const record = asRecord(value);
  const checks = asRecord(record.required_status_checks);
  const reviews = asRecord(record.required_pull_request_reviews);
  const bypass = asRecord(reviews.bypass_pull_request_allowances);
  const restrictions = asRecord(record.restrictions);
  return {
    required_status_checks: {
      strict: asBoolean(checks.strict),
      contexts: contexts(checks.checks ?? checks.contexts),
    },
    required_pull_request_reviews: {
      required_approving_review_count: asNumber(reviews.required_approving_review_count),
      bypass_pull_request_allowances: {
        users: names(bypass.users),
        teams: names(bypass.teams),
        apps: names(bypass.apps),
      },
    },
    enforce_admins: asBoolean(asRecord(record.enforce_admins).enabled ?? record.enforce_admins),
    required_conversation_resolution: asBoolean(asRecord(record.required_conversation_resolution).enabled ?? record.required_conversation_resolution),
    required_linear_history: asBoolean(asRecord(record.required_linear_history).enabled ?? record.required_linear_history),
    allow_force_pushes: asBoolean(record.allow_force_pushes),
    allow_deletions: asBoolean(record.allow_deletions),
  };
};

const equal = (left: unknown, right: unknown): boolean => JSON.stringify(left) === JSON.stringify(right);

const drift = (drifts: PolicyDrift[], field: string, expected: unknown, observed: unknown): void => {
  if (!equal(expected, observed)) drifts.push({ field, expected, observed });
};

export const findPolicyDrifts = (state: PolicyState): PolicyDrift[] => {
  const drifts: PolicyDrift[] = [];
  drift(drifts, 'repository.default_branch', expectedRepositoryPolicy.default_branch, state.repository.default_branch);
  drift(drifts, 'repository.allow_squash_merge', expectedRepositoryPolicy.allow_squash_merge, state.repository.allow_squash_merge);
  drift(drifts, 'repository.allow_merge_commit', expectedRepositoryPolicy.allow_merge_commit, state.repository.allow_merge_commit);
  drift(drifts, 'repository.allow_rebase_merge', expectedRepositoryPolicy.allow_rebase_merge, state.repository.allow_rebase_merge);
  if (state.protection === null) {
    drifts.push({ field: 'main.protection', expected: expectedProtectionPolicy, observed: null });
    return drifts;
  }
  drift(drifts, 'main.required_status_checks', expectedProtectionPolicy.required_status_checks, state.protection.required_status_checks);
  drift(drifts, 'main.required_pull_request_reviews', expectedProtectionPolicy.required_pull_request_reviews, state.protection.required_pull_request_reviews);
  drift(drifts, 'main.enforce_admins', expectedProtectionPolicy.enforce_admins, state.protection.enforce_admins);
  drift(drifts, 'main.required_conversation_resolution', expectedProtectionPolicy.required_conversation_resolution, state.protection.required_conversation_resolution);
  drift(drifts, 'main.required_linear_history', expectedProtectionPolicy.required_linear_history, state.protection.required_linear_history);
  drift(drifts, 'main.allow_force_pushes', expectedProtectionPolicy.allow_force_pushes, state.protection.allow_force_pushes);
  drift(drifts, 'main.allow_deletions', expectedProtectionPolicy.allow_deletions, state.protection.allow_deletions);
  return drifts;
};

export const policyState = (repository: unknown, protection: unknown): PolicyState => ({
  repository: normalizeRepository(repository),
  protection: normalizeProtection(protection),
});

const repositoryPath = (repository: string): string => `/repos/${repository}`;

const protectionPath = (repository: string, branch: string): string => `${repositoryPath(repository)}/branches/${encodeURIComponent(branch)}/protection`;

const protectionRequest = (): Request => ({
  method: 'PUT',
  body: JSON.stringify({
    required_status_checks: { strict: true, contexts: [] },
    enforce_admins: true,
    required_pull_request_reviews: {
      dismiss_stale_reviews: false,
      require_code_owner_reviews: false,
      required_approving_review_count: 0,
      require_last_push_approval: false,
    },
    restrictions: null,
    required_linear_history: true,
    allow_force_pushes: false,
    allow_deletions: false,
    required_conversation_resolution: true,
  }),
});

export const applyPolicy = async (repository: string, branch: string, transport: Transport): Promise<void> => {
  await transport(repositoryPath(repository), {
    method: 'PATCH',
    body: JSON.stringify({
      default_branch: expectedRepositoryPolicy.default_branch,
      allow_squash_merge: true,
      allow_merge_commit: false,
      allow_rebase_merge: false,
    }),
  });
  await transport(protectionPath(repository, branch), protectionRequest());
  const state = await readPolicy(repository, branch, transport);
  const drifts = findPolicyDrifts(state);
  if (drifts.length > 0) throw new Error(`post-apply verification failed:\n${formatDrifts(drifts)}`);
};

export const readPolicy = async (repository: string, branch: string, transport: Transport): Promise<PolicyState> => {
  const repositoryResponse = await transport(repositoryPath(repository));
  let protectionResponse: unknown = null;
  try {
    protectionResponse = await transport(protectionPath(repository, branch));
  } catch (error) {
    if (!String(error).includes('404')) throw error;
  }
  return policyState(repositoryResponse, protectionResponse);
};

export const formatDrifts = (drifts: readonly PolicyDrift[]): string => drifts
  .map(({ field, expected, observed }) => `${field}: expected=${JSON.stringify(expected)} observed=${JSON.stringify(observed)}`)
  .join('\n');

const token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN ?? '';

export const redactOutput = (value: string, secret = token): string => secret === '' ? value : value.split(secret).join('[REDACTED]');

const githubTransport: Transport = async (path, request = {}) => {
  const response = await fetch(`${API_ROOT}${path}`, {
    method: request.method ?? 'GET',
    headers: { Accept: ACCEPT, 'X-GitHub-Api-Version': '2022-11-28', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    body: request.body,
  });
  const body = await response.text();
  if (!response.ok) throw new Error(redactOutput(`GitHub ${response.status} ${response.statusText}: ${body}`));
  return body === '' ? null : JSON.parse(body) as unknown;
};

const usage = (): never => {
  throw new Error('usage: repository-policy.ts [--apply] cgasgarth/RustTable main');
};

export const run = async (args: readonly string[], transport: Transport = githubTransport): Promise<void> => {
  const applying = args[0] === APPLY_FLAG;
  const values = applying ? args.slice(1) : args;
  if (values.length !== 2 || values[0] !== 'cgasgarth/RustTable' || values[1] !== 'main') usage();
  const repository = values[0];
  const branch = values[1];
  if (applying) {
    await applyPolicy(repository, branch, transport);
    console.log('repository policy applied and verified: cgasgarth/RustTable main');
    return;
  }
  const drifts = findPolicyDrifts(await readPolicy(repository, branch, transport));
  if (drifts.length > 0) throw new Error(`repository policy drift:\n${formatDrifts(drifts)}`);
  console.log('repository policy compliant: cgasgarth/RustTable main');
};

if (import.meta.main) {
  await run(Bun.argv.slice(2)).catch((error: unknown) => {
    console.error(redactOutput(error instanceof Error ? error.message : String(error)));
    process.exitCode = 1;
  });
}
