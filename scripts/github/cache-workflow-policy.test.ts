import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';
import { findCacheWorkflowViolations } from './cache-workflow-policy';
import { fixtures } from './cache-workflow-policy.fixtures';

const readWorkflow = (name: string): Promise<string> => Bun.file(resolve(import.meta.dir, '../../.github/workflows', name)).text();

describe('cache workflow policy fixtures', () => {
  test('accepts restore-only PR and writer main workflows', () => {
    expect(findCacheWorkflowViolations(fixtures.compliantPrWorkflow, fixtures.compliantMainWorkflow)).toEqual([]);
  });

  test('rejects every PR cache-save path', () => {
    expect(findCacheWorkflowViolations(fixtures.prWriter, fixtures.compliantMainWorkflow).map(({ message }) => message)).toContain(
      'PR workflow must not use the cache writer action',
    );
    expect(findCacheWorkflowViolations(fixtures.prSaver, fixtures.compliantMainWorkflow).map(({ message }) => message)).toContain(
      'PR workflow must not use the cache save action',
    );
  });

  test('requires the stable main cache prefix for PR restore', () => {
    expect(findCacheWorkflowViolations(fixtures.prWrongPrefix, fixtures.compliantMainWorkflow).map(({ message }) => message)).toContain(
      'PR workflow must use the stable rust-main restore prefix',
    );
  });

  test('requires main to remain a cache writer', () => {
    expect(findCacheWorkflowViolations(fixtures.compliantPrWorkflow, fixtures.mainRestoreOnly).map(({ message }) => message)).toContain(
      'main workflow must use the cache writer action actions/cache@5a3ec84eff668545956fd18022155c47e93e2684',
    );
  });

  test('accepts the checked-in workflows', async () => {
    const [prWorkflow, mainWorkflow] = await Promise.all([
      readWorkflow('rust-pr.yml'),
      readWorkflow('rust-main.yml'),
    ]);
    expect(findCacheWorkflowViolations(prWorkflow, mainWorkflow)).toEqual([]);
  });
});
