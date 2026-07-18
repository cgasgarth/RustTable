import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import {
  type CommandRequest,
  type CommandResult,
  cleanupRepositoryAppBundles,
  discoverRepositoryAppBundles,
  findStaleRepositoryRegistrationPaths,
  installCanonicalComputerUseApp,
  parseComputerUseInstallOptions,
  parseGitWorktreePaths,
  parseLaunchServicesRegistrations,
  pathExists,
  readBundleIdentifier,
  unregisterMissingRepositoryBundles,
} from './computer-use-app-install';
import {
  createRustTableBundle,
  resolveRustTableVersion,
  validateBundle,
} from './rusttable-app-bundle';

const releaseBundlePath = (root: string): string =>
  join(root, 'target/release/bundle/macos/RustTable.app');

const help = `Usage: bun run install:computer-use [options]

Build, install, and register RustTable.app for Computer Use.

Options:
  --app-path PATH  Install into PATH (default: ~/Applications/RustTable.app)
  --compact        Reduce build output
  --no-build       Use the existing release bundle
  --no-install     Build/validate without changing the canonical install
  --no-launch      Do not open the installed app
  -h, --help       Show this help
`;

const runCommand = async (request: CommandRequest): Promise<CommandResult> => {
  const child = Bun.spawn([request.command, ...request.args], { stderr: 'pipe', stdout: 'pipe' });
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
  });
  await validateBundle(appPath, join(root, 'LICENSE'));
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
    await installCanonicalComputerUseApp({
      installPath: options.installPath,
      run: runCommand,
      sourcePath: bundlePath,
      transactionId: randomUUID(),
    });
    const worktreeResult = await runCommand({
      args: ['worktree', 'list', '--porcelain', '-z'],
      command: 'git',
      label: 'list RustTable worktrees',
    });
    const worktreePaths = parseGitWorktreePaths(worktreeResult.stdout);
    const bundlePaths = await discoverRepositoryAppBundles(worktreePaths);
    const removed = await cleanupRepositoryAppBundles({
      bundlePaths,
      keepPaths: [bundlePath, options.installPath],
      run: runCommand,
    });
    const registrations = await runCommand({
      args: ['-dump'],
      command: '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister',
      label: 'inspect LaunchServices registrations',
    });
    const stalePaths = findStaleRepositoryRegistrationPaths({
      canonicalPath: options.installPath,
      registrations: parseLaunchServicesRegistrations(registrations.stdout),
      worktreePaths,
    });
    const unregistered = await unregisterMissingRepositoryBundles({ paths: stalePaths, run: runCommand });
    process.stdout.write(`Installed ${options.installPath}; cleaned ${removed.length} bundle(s), unregistered ${unregistered.length} stale path(s).\n`);
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
