import { describe, expect, test } from 'bun:test';
import { fileURLToPath } from 'node:url';

import { loadPlatformTargets, validatePlatformTargets } from './platform-support';

describe('platform support contract', () => {
  test('loads the three supported product targets in contract order', async () => {
    const targets = await loadPlatformTargets(fileURLToPath(new URL('..', import.meta.url)));
    expect(targets.map((target) => target.triple)).toEqual([
      'x86_64-unknown-linux-gnu',
      'aarch64-apple-darwin',
      'x86_64-pc-windows-msvc',
    ]);
  });

  test('rejects duplicate or incomplete target declarations', () => {
    expect(() => validatePlatformTargets({ schema_version: 1, targets: [{ triple: 'x', os: '', architecture: 'x', runner: 'x' }] })).toThrow();
    expect(() => validatePlatformTargets({ schema_version: 1, targets: [
      { triple: 'x', os: 'linux', architecture: 'x', runner: 'x' },
      { triple: 'x', os: 'linux', architecture: 'x', runner: 'x' },
    ] })).toThrow();
  });
});
