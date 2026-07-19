import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';
import { findCacheWorkflowViolations } from './cache-workflow-policy';
import { fixtures } from './cache-workflow-policy.fixtures';

const readWorkflow = (name: string): Promise<string> => Bun.file(resolve(import.meta.dir, '../../.github/workflows', name)).text();

describe('cache workflow policy fixtures', () => {
  test('accepts the main writer workflow', () => {
    expect(findCacheWorkflowViolations(fixtures.compliantMainWorkflow)).toEqual([]);
  });

  test('rejects restore-only main validation', () => {
    expect(findCacheWorkflowViolations(fixtures.mainRestoreOnly).map(({ message }) => message)).toContain(
      'main workflow must not use the restore-only cache action',
    );
  });

  test('requires the main cache key', () => {
    expect(findCacheWorkflowViolations(fixtures.mainMissingKey).map(({ message }) => message)).toContain(
      'main workflow must publish the compatible rust-main cache key',
    );
  });

  test('accepts the checked-in workflows', async () => {
    const mainWorkflow = await readWorkflow('rust-main.yml');
    expect(findCacheWorkflowViolations(mainWorkflow)).toEqual([]);
  });
});
