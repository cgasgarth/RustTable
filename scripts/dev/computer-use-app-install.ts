import { access, lstat, mkdir, readdir, rename, rm } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { basename, dirname, join, resolve } from 'node:path';
import {
  RUSTTABLE_BUNDLE_IDENTIFIER,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
  parseBundleIdentifier,
  readBundleManifest,
  validateBundle,
  type BundleManifest,
} from './rusttable-app-bundle';

export {
  RUSTTABLE_BUNDLE_IDENTIFIER,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER,
} from './rusttable-app-bundle';
export const RUSTTABLE_LEGACY_BUNDLE_IDENTIFIER = RUSTTABLE_BUNDLE_IDENTIFIER;
export const DEFAULT_COMPUTER_USE_APP_PATH = join(
  process.env.HOME ?? '/tmp',
  'Applications',
  `${RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY.displayName}.app`,
);

const LAUNCH_SERVICES =
  '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister';
const REPOSITORY_BUNDLE_IDENTIFIERS = new Set([
  RUSTTABLE_BUNDLE_IDENTIFIER,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER,
]);
const TARGET_BUNDLE_DIRECTORIES = [
  'target/debug/bundle/macos',
  'target/release/bundle/macos',
] as const;

export interface CommandRequest {
  allowedExitCodes?: readonly number[];
  args: string[];
  command: string;
  label: string;
}

export interface CommandResult {
  exitCode: number;
  stderr: string;
  stdout: string;
}

export type CommandRunner = (request: CommandRequest) => Promise<CommandResult>;
export type BundleIdentifierReader = (bundlePath: string) => Promise<string>;

export interface ComputerUseInstallOptions {
  installPath: string;
  shouldBuild: boolean;
  shouldInstall: boolean;
  shouldLaunch: boolean;
  showHelp: boolean;
  verboseBuildLogs: boolean;
}

export interface LaunchServicesRegistration {
  bundleIdentifier: string;
  path: string;
}

export const parseComputerUseInstallOptions = (
  args: readonly string[],
  cwd = process.cwd(),
): ComputerUseInstallOptions => {
  const knownFlags = new Set([
    '--app-path',
    '--compact',
    '--help',
    '-h',
    '--no-build',
    '--no-install',
    '--no-launch',
  ]);
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === undefined) continue;
    if (!knownFlags.has(argument)) throw new Error(`Unknown computer-use install option: ${argument}`);
    if (argument === '--app-path') throw new Error('Computer Use install path is fixed and cannot be overridden.');
  }

  return {
    installPath: resolve(cwd, DEFAULT_COMPUTER_USE_APP_PATH),
    shouldBuild: !args.includes('--no-build'),
    shouldInstall: !args.includes('--no-install'),
    shouldLaunch: !args.includes('--no-launch'),
    showHelp: args.includes('--help') || args.includes('-h'),
    verboseBuildLogs: !args.includes('--compact'),
  };
};

export const parseGitWorktreePaths = (porcelain: string): string[] =>
  porcelain
    .split('\0')
    .filter((field) => field.startsWith('worktree '))
    .map((field) => resolve(field.slice('worktree '.length)))
    .filter((path, index, paths) => paths.indexOf(path) === index);

export { parseBundleIdentifier };

export const readBundleIdentifier: BundleIdentifierReader = async (bundlePath) =>
  (await readBundleManifest(bundlePath)).CFBundleIdentifier;

type BundleManifestReader = (bundlePath: string) => Promise<BundleManifest>;

const assertCompleteBundle = async (
  bundlePath: string,
  readIdentifier: BundleIdentifierReader,
  readManifest: BundleManifestReader,
): Promise<void> => {
  await validateBundle(bundlePath, undefined, RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY);
  const manifest = await readManifest(bundlePath);
  const identifier = await readIdentifier(bundlePath);
  const expected = RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY;
  if (
    manifest.CFBundleDisplayName !== expected.displayName ||
    manifest.CFBundleName !== expected.bundleName ||
    manifest.CFBundleIdentifier !== expected.bundleIdentifier ||
    identifier !== expected.bundleIdentifier
  ) {
    throw new Error(`Refusing RustTable app mutation for ${bundlePath}: unexpected bundle identifier ${identifier}.`);
  }
};

export const pathExists = async (path: string): Promise<boolean> => {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
};

export const discoverRepositoryAppBundles = async (worktreePaths: readonly string[]): Promise<string[]> => {
  const bundles: string[] = [];
  for (const worktreePath of worktreePaths) {
    for (const relativeDirectory of TARGET_BUNDLE_DIRECTORIES) {
      const directory = join(worktreePath, relativeDirectory);
      const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
      for (const entry of entries) {
        if (entry.isDirectory() && entry.name.endsWith('.app')) bundles.push(resolve(directory, entry.name));
      }
    }
  }
  return bundles.filter((path, index) => bundles.indexOf(path) === index).sort();
};

export const parseLaunchServicesRegistrations = (dump: string): LaunchServicesRegistration[] => {
  const registrations: LaunchServicesRegistration[] = [];
  let path: string | undefined;
  for (const line of dump.split('\n')) {
    const pathMatch = /^path:\s+(.+?)\s+\(0x[0-9a-f]+\)$/i.exec(line);
    if (pathMatch?.[1] !== undefined) path = resolve(pathMatch[1]);
    const identifierMatch = /^identifier:\s+(\S+)\s*$/.exec(line);
    if (path !== undefined && identifierMatch?.[1] !== undefined) {
      registrations.push({ bundleIdentifier: identifierMatch[1], path });
      path = undefined;
    }
  }
  return registrations;
};

const isWorktreeTargetBundle = (path: string, worktreePaths: readonly string[]): boolean => {
  const normalizedPath = resolve(path);
  const targetSuffix = /\/target\/(?:debug|release)\/bundle\/macos\/[^/]+\.app$/;
  return targetSuffix.test(normalizedPath) && worktreePaths.some((worktreePath) =>
    normalizedPath.startsWith(`${resolve(worktreePath)}/`),
  );
};

const isRepositoryOwnedBundlePath = (
  path: string,
  worktreePaths: readonly string[],
  exactPaths: readonly string[],
): boolean =>
  isWorktreeTargetBundle(path, worktreePaths) || exactPaths.some((exactPath) => resolve(exactPath) === resolve(path));

export const findStaleRepositoryRegistrationPaths = ({
  canonicalPath,
  legacyPaths = [],
  registrations,
  worktreePaths,
}: {
  canonicalPath: string;
  legacyPaths?: readonly string[];
  registrations: readonly LaunchServicesRegistration[];
  worktreePaths: readonly string[];
}): string[] => {
  const canonical = resolve(canonicalPath);
  return registrations
    .filter((registration) => REPOSITORY_BUNDLE_IDENTIFIERS.has(registration.bundleIdentifier))
    .map((registration) => resolve(registration.path))
    .filter((path) => path !== canonical)
    .filter((path) => isRepositoryOwnedBundlePath(path, worktreePaths, legacyPaths))
    .filter((path, index, paths) => paths.indexOf(path) === index)
    .sort();
};

const unregisterBundle = async (bundlePath: string, run: CommandRunner): Promise<void> => {
  await run({
    allowedExitCodes: [0, 1],
    args: ['-u', bundlePath],
    command: LAUNCH_SERVICES,
    label: `unregister ${bundlePath}`,
  });
};

const registerBundle = async (bundlePath: string, run: CommandRunner): Promise<void> => {
  await run({
    args: ['-f', bundlePath],
    command: LAUNCH_SERVICES,
    label: `register ${bundlePath}`,
  });
};

export const installCanonicalComputerUseApp = async ({
  installPath,
  readManifest,
  readIdentifier = readBundleIdentifier,
  run,
  sourcePath,
  transactionId,
}: {
  installPath: string;
  readManifest?: BundleManifestReader;
  readIdentifier?: BundleIdentifierReader;
  run: CommandRunner;
  sourcePath: string;
  transactionId: string;
}): Promise<void> => {
  const readManifestValue = readManifest ?? readBundleManifest;
  await assertCompleteBundle(sourcePath, readIdentifier, readManifestValue);
  await mkdir(dirname(installPath), { recursive: true });
  const transactionPrefix = join(dirname(installPath), `.${basename(installPath)}.${transactionId}`);
  const stagingPath = `${transactionPrefix}.stage`;
  const backupPath = `${transactionPrefix}.backup`;
  await rm(stagingPath, { force: true, recursive: true });
  await rm(backupPath, { force: true, recursive: true });

  let backupCreated = false;
  let newBundleInstalled = false;
  try {
    await run({ args: [sourcePath, stagingPath], command: 'ditto', label: 'stage computer-use app' });
    await assertCompleteBundle(stagingPath, readIdentifier, readManifestValue);
    await run({ allowedExitCodes: [0, 1], args: ['-x', 'RustTable'], command: 'pkill', label: 'quit RustTable' });
    if (await pathExists(installPath)) {
      await assertCompleteBundle(installPath, readIdentifier, readManifestValue);
      await unregisterBundle(installPath, run);
      await rename(installPath, backupPath);
      backupCreated = true;
    }
    await rename(stagingPath, installPath);
    newBundleInstalled = true;
    await registerBundle(installPath, run);
    if (backupCreated) await rm(backupPath, { force: true, recursive: true });
  } catch (error) {
    if (backupCreated) {
      if (newBundleInstalled) await rm(installPath, { force: true, recursive: true });
      await rename(backupPath, installPath);
      await registerBundle(installPath, run).catch(() => undefined);
    }
    throw error;
  } finally {
    await rm(stagingPath, { force: true, recursive: true });
  }
};

const moveBundleToRecovery = async (bundlePath: string, recoveryDirectory: string): Promise<string> => {
  await mkdir(recoveryDirectory, { recursive: true });
  const recoveryPath = join(
    recoveryDirectory,
    `${basename(bundlePath, '.app')}-${randomUUID()}.app`,
  );
  await rename(bundlePath, recoveryPath);
  return recoveryPath;
};

export const cleanupRepositoryAppBundles = async ({
  bundlePaths,
  keepPaths,
  recoveryDirectory = join(process.env.HOME ?? '/tmp', '.Trash'),
  repositoryPaths = [],
  worktreePaths = [],
  readIdentifier = readBundleIdentifier,
  run,
}: {
  bundlePaths: readonly string[];
  keepPaths: readonly string[];
  recoveryDirectory?: string;
  repositoryPaths?: readonly string[];
  worktreePaths?: readonly string[];
  readIdentifier?: BundleIdentifierReader;
  run: CommandRunner;
}): Promise<string[]> => {
  const keep = new Set(keepPaths.map((path) => resolve(path)));
  const removed: string[] = [];
  for (const bundlePath of bundlePaths) {
    const resolvedPath = resolve(bundlePath);
    if (keep.has(resolvedPath)) continue;
    if (!isRepositoryOwnedBundlePath(resolvedPath, worktreePaths, repositoryPaths)) continue;
    if ((await lstat(resolvedPath).catch(() => undefined))?.isSymbolicLink()) continue;
    const identifier = await readIdentifier(resolvedPath).catch(() => 'unreadable');
    if (!REPOSITORY_BUNDLE_IDENTIFIERS.has(identifier)) continue;
    try {
      await validateBundle(resolvedPath);
    } catch {
      continue;
    }
    await unregisterBundle(resolvedPath, run);
    await moveBundleToRecovery(resolvedPath, recoveryDirectory);
    removed.push(resolvedPath);
  }
  return removed;
};

export const unregisterMissingRepositoryBundles = async ({
  paths,
  run,
}: {
  paths: readonly string[];
  run: CommandRunner;
}): Promise<string[]> => {
  const unregistered: string[] = [];
  for (const path of paths) {
    if (await pathExists(path)) continue;
    await unregisterBundle(path, run);
    unregistered.push(path);
  }
  return unregistered;
};

export const unregisterRepositoryBundles = async ({
  paths,
  run,
}: {
  paths: readonly string[];
  run: CommandRunner;
}): Promise<string[]> => {
  for (const path of paths) await unregisterBundle(path, run);
  return [...paths];
};
