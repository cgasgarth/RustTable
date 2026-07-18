import { resolve } from 'node:path';

export const CACHE_ACTION_PIN = '5a3ec84eff668545956fd18022155c47e93e2684';
export const CACHE_RESTORE_ACTION = `actions/cache/restore@${CACHE_ACTION_PIN}`;
export const CACHE_WRITER_ACTION = `actions/cache@${CACHE_ACTION_PIN}`;
export const RUST_MAIN_CACHE_KEY = "rust-main-${{ runner.os }}-rust-1.95.0-${{ hashFiles('Cargo.lock', 'package.json') }}";
export const RUST_MAIN_WRITER_CACHE_KEY = "rust-main-Linux-rust-1.95.0-${{ hashFiles('Cargo.lock', 'package.json') }}";
export const RUST_MAIN_CACHE_PREFIX = 'rust-main-${{ runner.os }}-rust-1.95.0-';

export type CacheWorkflowViolation = {
  workflow: 'pr' | 'main';
  message: string;
};

const count = (text: string, pattern: RegExp): number => [...text.matchAll(pattern)].length;

const violation = (workflow: CacheWorkflowViolation['workflow'], message: string): CacheWorkflowViolation => ({
  workflow,
  message,
});

export const findCacheWorkflowViolations = (prWorkflow: string, mainWorkflow: string): CacheWorkflowViolation[] => {
  const violations: CacheWorkflowViolation[] = [];
  const prWriters = count(prWorkflow, /uses:\s*actions\/cache@/g);
  const prSavers = count(prWorkflow, /uses:\s*actions\/cache\/save@/g);
  const prRestores = count(prWorkflow, /uses:\s*actions\/cache\/restore@/g);
  const mainWriters = count(mainWorkflow, /uses:\s*actions\/cache@/g);

  if (prWriters > 0) violations.push(violation('pr', 'PR workflow must not use the cache writer action'));
  if (prSavers > 0) violations.push(violation('pr', 'PR workflow must not use the cache save action'));
  if (prRestores !== 1) violations.push(violation('pr', `PR workflow must use exactly one cache restore action (found ${prRestores})`));
  if (!prWorkflow.includes(`uses: ${CACHE_RESTORE_ACTION}`)) {
    violations.push(violation('pr', `PR workflow must use the immutable restore action ${CACHE_RESTORE_ACTION}`));
  }
  if (!prWorkflow.includes(`key: ${RUST_MAIN_CACHE_KEY}`)) {
    violations.push(violation('pr', 'PR workflow must use the compatible rust-main cache key'));
  }
  if (!prWorkflow.includes(`restore-keys: |\n            ${RUST_MAIN_CACHE_PREFIX}`)) {
    violations.push(violation('pr', 'PR workflow must use the stable rust-main restore prefix'));
  }

  if (mainWriters === 0) violations.push(violation('main', `main workflow must use the cache writer action ${CACHE_WRITER_ACTION}`));
  if (!mainWorkflow.includes(`uses: ${CACHE_WRITER_ACTION}`)) {
    violations.push(violation('main', `main workflow must use the immutable writer action ${CACHE_WRITER_ACTION}`));
  }
  if (!mainWorkflow.includes(`key: ${RUST_MAIN_WRITER_CACHE_KEY}`)) {
    violations.push(violation('main', 'main workflow must publish the compatible rust-main cache key'));
  }

  return violations;
};

export const formatCacheWorkflowViolations = (violations: readonly CacheWorkflowViolation[]): string => violations
  .map(({ workflow, message }) => `cache policy: ${workflow} workflow: ${message}`)
  .join('\n');

const repositoryRoot = resolve(import.meta.dir, '../..');
const readWorkflow = (name: string): Promise<string> => Bun.file(resolve(repositoryRoot, '.github/workflows', name)).text();

if (import.meta.main) {
  const [prWorkflow, mainWorkflow] = await Promise.all([
    readWorkflow('rust-pr.yml'),
    readWorkflow('rust-main.yml'),
  ]);
  const violations = findCacheWorkflowViolations(prWorkflow, mainWorkflow);
  if (violations.length > 0) {
    console.error(formatCacheWorkflowViolations(violations));
    process.exitCode = 1;
  } else {
    console.log('cache workflow policy: PR restore-only and main writer compliant');
  }
}
