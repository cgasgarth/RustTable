import { dirname, join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import {
  type CommandRequest,
  type CommandResult,
  cleanupRepositoryAppBundles,
  discoverApplicationBundles,
  discoverRepositoryAppBundles,
  findStaleRepositoryRegistrationPaths,
  installCanonicalComputerUseApp,
  parseComputerUseInstallOptions,
  parseGitWorktreePaths,
  parseLaunchServicesRegistrations,
  pathExists,
  readBundleIdentifier,
  unregisterRepositoryBundles,
} from './computer-use-app-install';
import {
  createRustTableBundle,
  RUSTTABLE_BUNDLE_IDENTITY,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
  resolveRustTableVersion,
  validateBundle,
} from './rusttable-app-bundle';

const releaseBundlePath = (root: string): string =>
  join(root, 'target/release/bundle/macos/RustTable.app');

const computerUseBundlePath = (root: string): string =>
  join(root, 'target/release/bundle/macos/rusttable - latest.app');

const help = `Usage: bun run install:computer-use [options]

Build, install, and register rusttable - latest.app for Computer Use.

Options:
  The canonical install path is ~/Applications/rusttable - latest.app.
  --compact        Reduce build output
  --no-build       Use the existing release bundle
  --no-install     Build/validate without changing the canonical install
  --no-launch      Do not open the installed app
  -h, --help       Show this help
`;

const runCommand = async (request: CommandRequest): Promise<CommandResult> => {
  const child = Bun.spawn([request.command, ...request.args], {
    env: { ...process.env, CARGO_BUILD_JOBS: '10' },
    stderr: 'pipe',
    stdout: 'pipe',
  });
  const [stdout, stderr] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
  ]);
  const result = { exitCode: await child.exited, stderr, stdout };
  if (!(request.allowedExitCodes ?? [0]).includes(result.exitCode)) {
    throw new Error(`${request.label} failed with exit code ${result.exitCode}: ${stderr.trim()}`);
  }
  return result;
};

const writeAppBundle = async (root: string, run: CommandRunner): Promise<string> => {
  const appPath = releaseBundlePath(root);
  const version = await resolveRustTableVersion(root, run);
  await createRustTableBundle({
    appPath,
    executablePath: join(root, 'target/release/rusttable-app'),
    licensePath: join(root, 'LICENSE'),
    version,
    identity: RUSTTABLE_BUNDLE_IDENTITY,
  });
  await validateBundle(appPath, join(root, 'LICENSE'));
  return appPath;
};

const writeComputerUseBundle = async (root: string, run: CommandRunner): Promise<string> => {
  const appPath = computerUseBundlePath(root);
  const version = await resolveRustTableVersion(root, run);
  await createRustTableBundle({
    appPath,
    executablePath: join(root, 'target/release/rusttable-app'),
    licensePath: join(root, 'LICENSE'),
    version,
    identity: RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
  });
  await validateBundle(appPath, join(root, 'LICENSE'), RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY);
  return appPath;
};

const main = async (): Promise<void> => {
  const options = parseComputerUseInstallOptions(process.argv.slice(2));
  if (options.showHelp) {
    process.stdout.write(help);
    return;
  }
  if (process.platform !== 'darwin') throw new Error('install:computer-use currently requires macOS LaunchServices.');

  const rootResult = await runCommand({
    args: ['rev-parse', '--show-toplevel'],
    command: 'git',
    label: 'find repository root',
  });
  const root = resolve(rootResult.stdout.trim());
  const bundlePath = releaseBundlePath(root);
  if (options.shouldBuild) {
    const buildResult = await runCommand({
      args: ['build', '--release', '--package', 'rusttable-app', '--bin', 'rusttable-app', '--locked'],
      command: 'cargo',
      label: 'build RustTable release',
    });
    if (options.verboseBuildLogs) {
      process.stdout.write(buildResult.stdout);
      process.stderr.write(buildResult.stderr);
    }
    await writeAppBundle(root, runCommand);
  } else if (!(await pathExists(bundlePath))) {
    throw new Error(`Release bundle not found at ${bundlePath}; remove --no-build.`);
  }
  await validateBundle(bundlePath, join(root, 'LICENSE'));
  await readBundleIdentifier(bundlePath);

  if (options.shouldInstall) {
    const sourcePath = await writeComputerUseBundle(root, runCommand);
    await installCanonicalComputerUseApp({
      installPath: options.installPath,
      run: runCommand,
      sourcePath,
      transactionId: randomUUID(),
    });
    const worktreeResult = await runCommand({
      args: ['worktree', 'list', '--porcelain', '-z'],
      command: 'git',
      label: 'list RustTable worktrees',
    });
    const worktreePaths = parseGitWorktreePaths(worktreeResult.stdout);
    const bundlePaths = await discoverRepositoryAppBundles(worktreePaths);
    const legacyInstallPath = join(dirname(options.installPath), 'RustTable.app');
    const applicationBundlePaths = await discoverApplicationBundles(dirname(options.installPath));
    const recoveryDirectory = join(process.env.HOME ?? '/tmp', '.Trash');
    const removed = await cleanupRepositoryAppBundles({
      bundlePaths: [...bundlePaths, ...applicationBundlePaths, legacyInstallPath],
      keepPaths: [sourcePath, options.installPath],
      repositoryPaths: [...applicationBundlePaths, legacyInstallPath],
      worktreePaths,
      run: runCommand,
    });
    const removedPaths = new Set(removed);
    let unregisteredCount = 0;
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const registrations = await runCommand({
        args: ['-dump'],
        command: '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister',
        label: 'inspect LaunchServices registrations',
      });
      const stalePaths = findStaleRepositoryRegistrationPaths({
        canonicalPath: options.installPath,
        legacyPaths: [...applicationBundlePaths, legacyInstallPath, ...bundlePaths],
        managedDirectories: [recoveryDirectory],
        managedRepositoryRoots: [join(dirname(root), 'fork')],
        registrations: parseLaunchServicesRegistrations(registrations.stdout),
        worktreePaths,
      });
      const unregistered = await unregisterRepositoryBundles({
        paths: stalePaths.filter((path) => !removedPaths.has(path)),
        run: runCommand,
      });
      unregisteredCount += unregistered.length;
      if (unregistered.length === 0) break;
    }
    process.stdout.write(`Installed ${options.installPath}; cleaned ${removed.length} bundle(s), unregistered ${unregisteredCount} stale path(s).\n`);
  }

  if (options.shouldLaunch) {
    await runCommand({
      args: ['-a', options.shouldInstall ? options.installPath : bundlePath],
      command: 'open',
      label: 'launch RustTable',
    });
  }
};

if (import.meta.main) {
  await main().catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${message}\n`);
    process.exit(1);
  });
}
