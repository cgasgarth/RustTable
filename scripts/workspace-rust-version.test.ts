import { describe, expect, test } from 'bun:test';
import { formatCargoFailure, validateWorkspace } from './workspace-rust-version';

const root = '/repo';
const packageManifest = (value: string): string => `[package]\nname = "${value}"\nrust-version.workspace = true\n`;

const fixture = (changes: {
  rootManifest?: string;
  members?: string[];
  packages?: Array<Record<string, unknown>>;
  manifests?: Record<string, string>;
  metadata?: string;
} = {}) => {
  const members = changes.members ?? ['one', 'two'];
  const packages = changes.packages ?? members.map((name) => ({
    name,
    id: `path+file:///repo/crates/${name}#0.1.0`,
    manifest_path: `/repo/crates/${name}/Cargo.toml`,
    rust_version: '1.98',
  }));
  const manifests = changes.manifests ?? Object.fromEntries(members.map((name) => [
    `/repo/crates/${name}/Cargo.toml`, packageManifest(name),
  ]));
  return {
    root,
    rootManifest: changes.rootManifest ?? '[workspace.package]\nrust-version = "1.98"\n',
    metadata: changes.metadata ?? JSON.stringify({ workspace_members: packages.map((entry) => entry.id), packages }),
    packageManifests: manifests,
  };
};

const errors = (changes: Parameters<typeof fixture>[0] = {}): string[] => validateWorkspace(fixture(changes));

describe('workspace rust-version checker fixtures', () => {
  test('accepts valid metadata and exact inheritance', () => {
    expect(errors()).toEqual([]);
  });

  test.each([
    ['malformed root field', '[workspace.package]\nrust-version = 1.98\n', 'exact stable major.minor'],
    ['pre-release', '[workspace.package]\nrust-version = "1.98.0-alpha"\n', 'exact stable major.minor'],
    ['range', '[workspace.package]\nrust-version = ">=1.98"\n', 'exact stable major.minor'],
    ['wildcard', '[workspace.package]\nrust-version = "1.*"\n', 'exact stable major.minor'],
  ])('%s', (_name, manifest, expected) => {
    expect(errors({ rootManifest: manifest }).join('\n')).toContain(expected);
  });

  test('rejects duplicate root fields', () => {
    expect(errors({ rootManifest: '[workspace.package]\nrust-version = "1.98"\nrust-version = "1.98"\n' }).join('\n'))
      .toContain('rust-version is duplicated');
  });

  test('rejects an empty workspace', () => {
    expect(errors({ metadata: JSON.stringify({ workspace_members: [], packages: [] }) }).join('\n'))
      .toContain('workspace package set is empty');
  });

  test('rejects malformed Cargo metadata JSON', () => {
    expect(errors({ metadata: '{not-json' })).toEqual(['cargo metadata: malformed JSON']);
  });

  test('reports missing and duplicate package entries', () => {
    const metadata = JSON.stringify({
      workspace_members: ['one', 'two'],
      packages: [
        { name: 'one', id: 'one', manifest_path: '/repo/crates/one/Cargo.toml', rust_version: '1.98' },
        { name: 'one-copy', id: 'one', manifest_path: '/repo/crates/one-copy/Cargo.toml', rust_version: '1.98' },
      ],
    });
    const output = errors({ metadata });
    expect(output.join('\n')).toContain('package entry one is duplicated');
    expect(output.join('\n')).toContain('workspace package two is missing');
  });

  test('rejects a package outside crates', () => {
    const metadata = JSON.stringify({
      workspace_members: ['one'],
      packages: [{ name: 'one', id: 'one', manifest_path: '/repo/outside/Cargo.toml', rust_version: '1.98' }],
    });
    expect(errors({ members: ['one'], metadata }).join('\n')).toContain('manifest must be crates/one/Cargo.toml');
  });

  test('a newly discovered package fails until it inherits the workspace version', () => {
    const manifests = {
      '/repo/crates/one/Cargo.toml': packageManifest('one'),
      '/repo/crates/two/Cargo.toml': packageManifest('two'),
      '/repo/crates/three/Cargo.toml': '[package]\nname = "three"\n',
    };
    expect(errors({ members: ['one', 'two', 'three'], manifests }).join('\n')).toContain(
      'package three: rust-version.workspace = true is missing',
    );
    manifests['/repo/crates/three/Cargo.toml'] = packageManifest('three');
    expect(errors({ members: ['one', 'two', 'three'], manifests })).toEqual([]);
  });

  test.each([
    ['missing inheritance', '[package]\nname = "one"\n', 'is missing'],
    ['package-local literal', '[package]\nname = "one"\nrust-version = "1.98"\n', 'package-local rust-version is forbidden'],
    ['false inheritance', '[package]\nname = "one"\nrust-version.workspace = false\n', 'must be boolean true'],
    ['non-boolean inheritance', '[package]\nname = "one"\nrust-version.workspace = "true"\n', 'must be boolean true'],
    ['duplicate inheritance', '[package]\nrust-version.workspace = true\nrust-version.workspace = true\n', 'is duplicated'],
  ])('%s', (_name, manifest, expected) => {
    expect(errors({ members: ['one'], manifests: { '/repo/crates/one/Cargo.toml': manifest } }).join('\n')).toContain(expected);
  });

  test('rejects metadata and manifest disagreement', () => {
    const metadata = JSON.stringify({
      workspace_members: ['one'],
      packages: [{ name: 'one', id: 'one', manifest_path: '/repo/crates/one/Cargo.toml', rust_version: null }],
    });
    expect(errors({ members: ['one'], metadata }).join('\n')).toContain('metadata rust_version must be 1.98');
  });

  test('reports simultaneous package violations in package-name order', () => {
    const output = errors({
      manifests: {
        '/repo/crates/one/Cargo.toml': '[package]\nrust-version = "1.98"\n',
        '/repo/crates/two/Cargo.toml': '[package]\nrust-version.workspace = false\n',
      },
    });
    expect(output.filter((entry) => entry.startsWith('package '))).toEqual([
      'package one: package-local rust-version is forbidden',
      'package one: rust-version.workspace = true is missing',
      'package two: rust-version.workspace must be boolean true',
    ]);
  });

  test('formats a Cargo failure without spawning a process', () => {
    expect(formatCargoFailure({ exitCode: 17, stderr: 'locked metadata unavailable\n' }))
      .toBe('cargo metadata failed with exit code 17: locked metadata unavailable');
  });
});
