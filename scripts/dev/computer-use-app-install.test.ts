import { describe, expect, test } from 'bun:test';
import {
  findStaleRepositoryRegistrationPaths,
  parseBundleIdentifier,
  parseComputerUseInstallOptions,
  parseGitWorktreePaths,
  parseLaunchServicesRegistrations,
  RUSTTABLE_BUNDLE_IDENTIFIER,
} from './computer-use-app-install';

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
});
