import { mkdtemp, rm } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import {
  assertCanonicalLaunchServicesRegistration,
  type CommandRequest,
  type CommandResult,
  cleanupRepositoryAppBundles,
  discoverApplicationBundles,
  discoverRepositoryAppBundles,
  findStaleRepositoryRegistrationPaths,
  installCanonicalComputerUseApp,
  launchComputerUseApp,
  parseComputerUseInstallOptions,
  parseGitWorktreePaths,
  parseLaunchServicesRegistrations,
  pathExists,
  readBundleIdentifier,
  unregisterRepositoryBundles,
} from './computer-use-app-install';
import {
  createRustTableBundle,
  RUSTTABLE_BUNDLE_IDENTIFIER,
  RUSTTABLE_BUNDLE_IDENTITY,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
  resolveRustTableVersion,
  validateBundle,
} from './rusttable-app-bundle';

const releaseBundlePath = (root: string): string =>
  join(root, 'target/release/bundle/macos/RustTable.app');

export const commandEnvironment = (): Record<string, string> => {
  const environment: Record<string, string> = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (key !== 'CARGO_BUILD_JOBS' && value !== undefined) environment[key] = value;
  }
  return environment;
};

const help = `Usage: bun run install:computer-use [options]

Build, install, and register rusttable - latest.app for Computer Use.
The default does not open or activate the installed app.

Options:
  The canonical install path is ~/Applications/rusttable - latest.app.
  --compact        Reduce build output
  --launch         Open a decorated window sized to the usable working area
  --no-build       Use the existing release bundle
  --no-install     Build/validate without changing the canonical install
  --no-launch      Compatibility alias for the non-launching default
  -h, --help       Show this help
`;

const runCommand = async (request: CommandRequest): Promise<CommandResult> => {
  const child = Bun.spawn([request.command, ...request.args], {
    env: commandEnvironment(),
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

const writeComputerUseBundle = async (
  root: string,
  run: CommandRunner,
): Promise<{ appPath: string; stagingDirectory: string }> => {
  const stagingDirectory = await mkdtemp(join(tmpdir(), 'rusttable-computer-use-'));
  const appPath = join(stagingDirectory, `${RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY.displayName}.app`);
  try {
    const version = await resolveRustTableVersion(root, run);
    await createRustTableBundle({
      appPath,
      executablePath: join(root, 'target/release/rusttable-app'),
      licensePath: join(root, 'LICENSE'),
      version,
      identity: RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
    });
    await validateBundle(appPath, join(root, 'LICENSE'), RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY);
    return { appPath, stagingDirectory };
  } catch (error) {
    await rm(stagingDirectory, { force: true, recursive: true });
    throw error;
  }
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
    const source = await writeComputerUseBundle(root, runCommand);
    try {
      await installCanonicalComputerUseApp({
        installPath: options.installPath,
        run: runCommand,
        sourcePath: source.appPath,
        transactionId: randomUUID(),
      });
    } finally {
      await rm(source.stagingDirectory, { force: true, recursive: true });
    }
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
      bundlePaths: [...applicationBundlePaths, legacyInstallPath],
      keepPaths: [options.installPath],
      repositoryPaths: [...applicationBundlePaths, legacyInstallPath],
      worktreePaths,
      run: runCommand,
    });
    const removedPaths = new Set(removed);
    let unregisteredCount = 0;
    let registrations: ReturnType<typeof parseLaunchServicesRegistrations> = [];
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const registrationDump = await runCommand({
        args: ['-dump'],
        command: '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister',
        label: 'inspect LaunchServices registrations',
      });
      registrations = parseLaunchServicesRegistrations(registrationDump.stdout);
      const stalePaths = findStaleRepositoryRegistrationPaths({
        canonicalPath: options.installPath,
        legacyPaths: [...applicationBundlePaths, legacyInstallPath, ...bundlePaths],
        managedDirectories: [recoveryDirectory],
        managedRepositoryRoots: [join(dirname(root), 'fork')],
        registrations,
        worktreePaths,
      });
      const unregistered = await unregisterRepositoryBundles({
        paths: stalePaths.filter((path) => !removedPaths.has(path)),
        run: runCommand,
      });
      unregisteredCount += unregistered.length;
      if (unregistered.length === 0) break;
    }
    assertCanonicalLaunchServicesRegistration({ canonicalPath: options.installPath, registrations });
    process.stdout.write(`Installed ${options.installPath}; cleaned ${removed.length} bundle(s), unregistered ${unregisteredCount} stale path(s).\n`);
  }

  if (options.shouldLaunch) {
    await launchComputerUseApp({
      appPath: options.shouldInstall ? options.installPath : bundlePath,
      bundleIdentifier: options.shouldInstall
        ? RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY.bundleIdentifier
        : RUSTTABLE_BUNDLE_IDENTIFIER,
      run: runCommand,
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
