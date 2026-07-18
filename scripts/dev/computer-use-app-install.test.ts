import { describe, expect, test } from 'bun:test';
import { cp, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import {
  findStaleRepositoryRegistrationPaths,
  installCanonicalComputerUseApp,
  parseBundleIdentifier,
  parseComputerUseInstallOptions,
  parseGitWorktreePaths,
  parseLaunchServicesRegistrations,
  RUSTTABLE_BUNDLE_IDENTIFIER,
} from './computer-use-app-install';
import { createRustTableBundle, renderBundlePlist } from './rusttable-app-bundle';

describe('computer-use installer parsing', () => {
  test('parses defaults and supported flags', () => {
    const options = parseComputerUseInstallOptions(
      ['--app-path', '/Applications/RustTable.app', '--compact', '--no-build', '--no-launch'],
      '/repo',
    );
    expect(options).toEqual({
      installPath: '/Applications/RustTable.app',
      shouldBuild: false,
      shouldInstall: true,
      shouldLaunch: false,
      showHelp: false,
      verboseBuildLogs: false,
    });
  });

  test('rejects unknown flags and missing app paths', () => {
    expect(() => parseComputerUseInstallOptions(['--wat'])).toThrow('Unknown computer-use install option');
    expect(() => parseComputerUseInstallOptions(['--app-path'])).toThrow('--app-path requires a path value');
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
        'path: /repo/target/debug/bundle/macos/RustTable.app (0x123)\nidentifier: com.cgasgarth.rusttable\n',
      ),
    ).toEqual([
      { bundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER, path: '/repo/target/debug/bundle/macos/RustTable.app' },
    ]);
  });

  test('filters stale registrations by identity and worktree location', () => {
    const worktreePaths = ['/repo', '/repo/worktrees/one'];
    expect(
      findStaleRepositoryRegistrationPaths({
        canonicalPath: '/Users/test/Applications/RustTable.app',
        registrations: [
          {
            bundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER,
            path: '/repo/worktrees/one/target/release/bundle/macos/RustTable.app',
          },
          { bundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER, path: '/Users/test/Applications/RustTable.app' },
          { bundleIdentifier: 'com.example.other', path: '/repo/target/debug/bundle/macos/Other.app' },
          { bundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER, path: '/tmp/RustTable.app' },
        ],
        worktreePaths,
      }),
    ).toEqual(['/repo/worktrees/one/target/release/bundle/macos/RustTable.app']);
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
});

const makeBundle = async (root: string, name: string): Promise<string> => {
  const directory = join(root, name);
  const executable = join(root, `${name}-binary`);
  const license = join(root, `${name}-LICENSE`);
  await writeFile(executable, '#!/bin/sh\n');
  await writeFile(license, 'license\n');
  return createRustTableBundle({
    appPath: directory,
    executablePath: executable,
    licensePath: license,
    version: '0.1.0',
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
