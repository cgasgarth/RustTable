import { describe, expect, test } from 'bun:test';
import { cp, lstat, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import {
  findStaleRepositoryRegistrationPaths,
  cleanupRepositoryAppBundles,
  installCanonicalComputerUseApp,
  parseBundleIdentifier,
  parseComputerUseInstallOptions,
  parseGitWorktreePaths,
  parseLaunchServicesRegistrations,
  RUSTTABLE_BUNDLE_IDENTIFIER,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER,
  DEFAULT_COMPUTER_USE_APP_PATH,
  unregisterMissingRepositoryBundles,
} from './computer-use-app-install';
import {
  createRustTableBundle,
  renderBundlePlist,
  RUSTTABLE_BUNDLE_IDENTITY,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
  readBundleManifest,
} from './rusttable-app-bundle';

describe('computer-use installer parsing', () => {
  test('parses defaults and supported flags', () => {
    const options = parseComputerUseInstallOptions(['--compact', '--no-build', '--no-launch'], '/repo');
    expect(options).toEqual({
      installPath: DEFAULT_COMPUTER_USE_APP_PATH,
      shouldBuild: false,
      shouldInstall: true,
      shouldLaunch: false,
      showHelp: false,
      verboseBuildLogs: false,
    });
  });

  test('rejects unknown flags and missing app paths', () => {
    expect(() => parseComputerUseInstallOptions(['--wat'])).toThrow('Unknown computer-use install option');
    expect(() => parseComputerUseInstallOptions(['--app-path', '/tmp/other.app'])).toThrow('fixed');
  });

  test('parses a bundle identifier from a plist', () => {
    expect(parseBundleIdentifier(`<key>CFBundleIdentifier</key>\n<string>${RUSTTABLE_BUNDLE_IDENTIFIER}</string>`)).toBe(
      RUSTTABLE_BUNDLE_IDENTIFIER,
    );
  });

  test('deduplicates worktree paths', () => {
    expect(parseGitWorktreePaths('worktree /repo\0HEAD x\0worktree /repo/wt\0worktree /repo\0')).toEqual([
      '/repo',
      '/repo/wt',
    ]);
  });

  test('parses LaunchServices path and identifier pairs', () => {
    expect(
      parseLaunchServicesRegistrations(
        'path: /repo/target/debug/bundle/macos/rusttable - latest.app (0x123)\nidentifier: com.cgasgarth.rusttable.latest\n',
      ),
    ).toEqual([
      { bundleIdentifier: RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER, path: '/repo/target/debug/bundle/macos/rusttable - latest.app' },
    ]);
  });

  test('filters stale registrations by identity and worktree location', () => {
    const worktreePaths = ['/repo', '/repo/worktrees/one'];
    expect(
      findStaleRepositoryRegistrationPaths({
        canonicalPath: '/Users/test/Applications/rusttable - latest.app',
        legacyPaths: ['/Users/test/Applications/RustTable.app'],
        registrations: [
          {
            bundleIdentifier: RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER,
            path: '/repo/worktrees/one/target/release/bundle/macos/rusttable - latest.app',
          },
          { bundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER, path: '/Users/test/Applications/RustTable.app' },
          { bundleIdentifier: 'com.example.other', path: '/repo/target/debug/bundle/macos/Other.app' },
          { bundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER, path: '/tmp/RustTable.app' },
        ],
        worktreePaths,
      }),
    ).toEqual([
      '/Users/test/Applications/RustTable.app',
      '/repo/worktrees/one/target/release/bundle/macos/rusttable - latest.app',
    ]);
  });

  test('removes only missing stale registrations after path filtering', async () => {
    const root = await mkdtemp(join(tmpdir(), 'rusttable-installer-'));
    try {
      const missing = join(root, 'repo/target/release/bundle/macos/RustTable.app');
      const existing = await makeBundle(root, 'existing');
      const calls: string[] = [];
      const unregistered = await unregisterMissingRepositoryBundles({
        paths: [missing, existing],
        run: async (request) => {
          calls.push(request.label);
          return { exitCode: 0, stderr: '', stdout: '' };
        },
      });
      expect(unregistered).toEqual([missing]);
      expect(calls).toEqual(['unregister ' + missing]);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  test('refuses an invalid source before invoking transactional commands', async () => {
    const root = await mkdtemp(join(tmpdir(), 'rusttable-installer-'));
    try {
      const source = await makeBundle(root, 'source');
      await writeFile(join(source, 'Contents/Info.plist'), invalidManifest());
      const calls: string[] = [];
      await expect(
        installCanonicalComputerUseApp({
          installPath: join(root, 'Applications/RustTable.app'),
          run: async (request) => {
            calls.push(request.label);
            return { exitCode: 0, stderr: '', stdout: '' };
          },
          sourcePath: source,
          transactionId: 'source-invalid',
        }),
      ).rejects.toThrow();
      expect(calls).toEqual([]);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  test('refuses invalid staged and installed bundles before registration or replacement', async () => {
    const root = await mkdtemp(join(tmpdir(), 'rusttable-installer-'));
    try {
      const source = await makeBundle(root, 'source');
      const invalidInstalled = await makeBundle(root, 'installed');
      await writeFile(join(invalidInstalled, 'Contents/Info.plist'), invalidManifest());
      const calls: string[] = [];
      const run = async (request: { args: string[]; label: string; command: string }) => {
        calls.push(request.label);
        if (request.label === 'stage computer-use app') await cp(request.args[0]!, request.args[1]!, { recursive: true });
        return { exitCode: 0, stderr: '', stdout: '' };
      };
      await expect(
        installCanonicalComputerUseApp({
          installPath: invalidInstalled,
          run,
          sourcePath: source,
          transactionId: 'installed-invalid',
        }),
      ).rejects.toThrow();
      expect(calls).toEqual(['stage computer-use app', 'quit RustTable']);
      expect(await readFile(join(invalidInstalled, 'Contents/Info.plist'), 'utf8')).toBe(invalidManifest());

      const stagedCalls: string[] = [];
      await expect(
        installCanonicalComputerUseApp({
          installPath: join(root, 'new-install/RustTable.app'),
          run: async (request) => {
            stagedCalls.push(request.label);
            if (request.label === 'stage computer-use app') {
              await cp(request.args[0]!, request.args[1]!, { recursive: true });
              await writeFile(join(request.args[1]!, 'Contents/Info.plist'), invalidManifest());
            }
            return { exitCode: 0, stderr: '', stdout: '' };
          },
          sourcePath: source,
          transactionId: 'staged-invalid',
        }),
      ).rejects.toThrow();
      expect(stagedCalls).toEqual(['stage computer-use app']);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  test('replaces the canonical app transactionally and rolls back registration failure', async () => {
    const root = await mkdtemp(join(tmpdir(), 'rusttable-installer-'));
    try {
      const installPath = join(root, 'Applications/rusttable - latest.app');
      const source = await makeBundle(root, 'source', '0.2.0');
      await makeBundle(join(root, 'Applications'), 'rusttable - latest.app', '0.1.0');
      const calls: string[] = [];
      const run = async (request: { args: string[]; label: string; command: string }) => {
        calls.push(request.label);
        if (request.label === 'stage computer-use app') await cp(request.args[0]!, request.args[1]!, { recursive: true });
        return { exitCode: 0, stderr: '', stdout: '' };
      };
      await installCanonicalComputerUseApp({ installPath, run, sourcePath: source, transactionId: 'replace' });
      expect((await readBundleManifest(installPath)).CFBundleShortVersionString).toBe('0.2.0');
      expect(calls.indexOf('unregister ' + installPath)).toBeLessThan(calls.indexOf('register ' + installPath));
      expect((await readdir(join(root, 'Applications'))).filter((entry) => entry.endsWith('.app'))).toEqual(['rusttable - latest.app']);

      const failedSource = await makeBundle(root, 'failed-source', '0.3.0');
      let registerAttempts = 0;
      await expect(
        installCanonicalComputerUseApp({
          installPath,
          run: async (request) => {
            if (request.label === 'stage computer-use app') await cp(request.args[0]!, request.args[1]!, { recursive: true });
            if (request.label.startsWith('register ')) {
              registerAttempts += 1;
              if (registerAttempts === 1) throw new Error('registration failed');
            }
            return { exitCode: 0, stderr: '', stdout: '' };
          },
          sourcePath: failedSource,
          transactionId: 'rollback',
        }),
      ).rejects.toThrow('registration failed');
      expect((await readBundleManifest(installPath)).CFBundleShortVersionString).toBe('0.2.0');
      expect((await readdir(join(root, 'Applications'))).filter((entry) => entry.endsWith('.app'))).toEqual(['rusttable - latest.app']);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  test('cleans repository-owned duplicates recoverably and refuses unrelated or symlink paths', async () => {
    const root = await mkdtemp(join(tmpdir(), 'rusttable-installer-'));
    try {
      const worktree = join(root, 'repo');
      const duplicate = await makeBundle(join(worktree, 'target/release/bundle/macos'), 'RustTable.app', '0.1.0', RUSTTABLE_BUNDLE_IDENTITY);
      const unrelated = await makeBundle(join(root, 'unrelated'), 'RustTable.app', '0.1.0', RUSTTABLE_BUNDLE_IDENTITY);
      const outsideBoundary = await makeBundle(join(root, 'repo-other/target/release/bundle/macos'), 'RustTable.app', '0.1.0', RUSTTABLE_BUNDLE_IDENTITY);
      const symlinkPath = join(worktree, 'target/debug/bundle/macos/Symlink.app');
      await mkdir(join(worktree, 'target/debug/bundle/macos'), { recursive: true });
      await symlink(duplicate, symlinkPath);
      const recoveryDirectory = join(root, 'Trash');
      const labels: string[] = [];
      const removed = await cleanupRepositoryAppBundles({
        bundlePaths: [duplicate, unrelated, outsideBoundary, symlinkPath],
        keepPaths: [],
        recoveryDirectory,
        run: async (request) => {
          labels.push(request.label);
          return { exitCode: 0, stderr: '', stdout: '' };
        },
        worktreePaths: [worktree],
      });
      expect(removed).toEqual([duplicate]);
      expect(labels).toEqual(['unregister ' + duplicate]);
      expect(await readdir(recoveryDirectory)).toHaveLength(1);
      expect(await readBundleManifest(unrelated)).toMatchObject({ CFBundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER });
      expect(await readBundleManifest(outsideBoundary)).toMatchObject({ CFBundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER });
      expect((await lstat(symlinkPath)).isSymbolicLink()).toBe(true);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });
});

const makeBundle = async (
  root: string,
  name: string,
  version = '0.1.0',
  identity = RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
): Promise<string> => {
  await mkdir(root, { recursive: true });
  const directory = join(root, name);
  const executable = join(root, `${name}-binary`);
  const license = join(root, `${name}-LICENSE`);
  await writeFile(executable, '#!/bin/sh\n');
  await writeFile(license, 'license\n');
  return createRustTableBundle({
    appPath: directory,
    executablePath: executable,
    licensePath: license,
    version,
    identity,
  });
};

const invalidManifest = (): string =>
  renderBundlePlist({
    CFBundleDisplayName: 'NotRustTable',
    CFBundleExecutable: 'RustTable',
    CFBundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER,
    CFBundleName: 'RustTable',
    CFBundlePackageType: 'APPL',
    CFBundleShortVersionString: '0.1.0',
    CFBundleVersion: '0.1.0',
  });
