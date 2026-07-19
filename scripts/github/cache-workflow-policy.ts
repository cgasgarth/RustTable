import { resolve } from 'node:path';

export const CACHE_ACTION_PIN = '5a3ec84eff668545956fd18022155c47e93e2684';
export const CACHE_WRITER_ACTION = `actions/cache@${CACHE_ACTION_PIN}`;
export const RUST_BASELINE_CACHE_FRAGMENT = "rust-baseline-${{ hashFiles('rust-toolchain.toml', 'quality/compiler-baseline.toml') }}";
export const RUST_MAIN_WRITER_CACHE_KEY = "rust-main-Linux-" + RUST_BASELINE_CACHE_FRAGMENT + "-${{ hashFiles('Cargo.lock', 'package.json') }}";

export type CacheWorkflowViolation = {
  workflow: 'main';
  message: string;
};

const violation = (message: string): CacheWorkflowViolation => ({ workflow: 'main', message });

export const findCacheWorkflowViolations = (mainWorkflow: string): CacheWorkflowViolation[] => {
  const violations: CacheWorkflowViolation[] = [];
  const writers = [...mainWorkflow.matchAll(/uses:\s*actions\/cache@/g)].length;
  const restores = [...mainWorkflow.matchAll(/uses:\s*actions\/cache\/restore@/g)].length;

  if (writers === 0) violations.push(violation(`main workflow must use the cache writer action ${CACHE_WRITER_ACTION}`));
  if (restores > 0) violations.push(violation('main workflow must not use the restore-only cache action'));
  if (!mainWorkflow.includes(`uses: ${CACHE_WRITER_ACTION}`)) {
    violations.push(violation(`main workflow must use the immutable writer action ${CACHE_WRITER_ACTION}`));
  }
  if (!mainWorkflow.includes(`key: ${RUST_MAIN_WRITER_CACHE_KEY}`)) {
    violations.push(violation('main workflow must publish the compatible rust-main cache key'));
  }

  return violations;
};

export const formatCacheWorkflowViolations = (violations: readonly CacheWorkflowViolation[]): string => violations
  .map(({ workflow, message }) => `cache policy: ${workflow} workflow: ${message}`)
  .join('\n');

const repositoryRoot = resolve(import.meta.dir, '../..');

if (import.meta.main) {
  const mainWorkflow = await Bun.file(resolve(repositoryRoot, '.github/workflows/rust-main.yml')).text();
  const violations = findCacheWorkflowViolations(mainWorkflow);
  if (violations.length > 0) {
    console.error(formatCacheWorkflowViolations(violations));
    process.exitCode = 1;
  } else {
    console.log('cache workflow policy: main writer compliant; no PR workflow is permitted');
  }
}
