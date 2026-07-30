export const MACOS_DISTRIBUTION_SCHEMA = 'RUSTTABLE_MACOS_DISTRIBUTION_V2' as const;
export const MACOS_BUNDLE_IDENTIFIER = 'com.cgasgarth.rusttable' as const;

export type MacosHost = 'aarch64-apple-darwin' | 'x86_64-apple-darwin';
export type MachOArchitecture = 'arm64' | 'x86_64';

export type MacosArtifactIdentity = {
  schema: typeof MACOS_DISTRIBUTION_SCHEMA;
  packageVersion: string;
  archiveBasename: string;
  checksumBasename: string;
  rustRelease: string;
  rustHost: MacosHost;
  expectedMachOArchitecture: MachOArchitecture;
  observedMachOArchitecture: MachOArchitecture;
};

export type MacosArtifactIdentityInput = {
  rustcVersionOutput: string;
  lipoArchsOutput?: string;
  lipoOutput?: string;
  packageVersion: string;
  archiveBasename?: string;
};

const versionPattern = /^(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){0,2}$/;
const acceptedReleasePattern = /^[0-9]+\.[0-9]+\.[0-9]+(?:-beta\.[0-9]+)?$/;

const field = (output: string, name: string): string => {
  const values = output
    .split(/\r?\n/)
    .flatMap((line) => {
      const match = new RegExp(`^\\s*${name}:\\s*(.*?)\\s*$`).exec(line);
      return match?.[1] === undefined ? [] : [match[1]];
    });
  if (values.length !== 1 || values[0] === undefined || values[0] === '') {
    throw new Error(`rustc -vV must contain exactly one ${name} field`);
  }
  return values[0];
};

export const parseRustcVersion = (output: string): { release: string; host: MacosHost } => {
  const release = field(output, 'release');
  if (!acceptedReleasePattern.test(release)) throw new Error(`rustc release is not an accepted stable/beta release: ${release}`);
  const host = field(output, 'host');
  if (host !== 'aarch64-apple-darwin' && host !== 'x86_64-apple-darwin') {
    throw new Error(`unexpected rustc host: ${host}`);
  }
  return { release, host };
};

export const expectedMachOArchitecture = (host: string): MachOArchitecture => {
  if (host === 'aarch64-apple-darwin') return 'arm64';
  if (host === 'x86_64-apple-darwin') return 'x86_64';
  throw new Error(`unexpected macOS Rust target: ${host}`);
};

export const parseLipoArchs = (output: string): MachOArchitecture => {
  const tokens = output.trim() === '' ? [] : output.trim().split(/\s+/);
  if (tokens.length !== 1) throw new Error('lipo -archs must report exactly one Mach-O architecture');
  const architecture = tokens[0];
  if (architecture !== 'arm64' && architecture !== 'x86_64') {
    throw new Error(`unknown Mach-O architecture: ${architecture}`);
  }
  if (tokens.filter((token) => token === architecture).length !== 1) {
    throw new Error(`duplicate Mach-O architecture: ${architecture}`);
  }
  return architecture;
};

export const parseLipoArchitectures = parseLipoArchs;

export const validateCargoPackageVersion = (version: string): string => {
  if (!versionPattern.test(version) || version.length > 18) {
    throw new Error(`invalid Cargo package version: ${version}`);
  }
  return version;
};

export const buildMacosArchiveBasename = (packageVersion: string, rustHost: MacosHost): string => {
  const version = validateCargoPackageVersion(packageVersion);
  if (rustHost !== 'aarch64-apple-darwin' && rustHost !== 'x86_64-apple-darwin') {
    throw new Error(`unexpected macOS Rust target: ${rustHost}`);
  }
  return `RustTable-${version}-${rustHost}-unsigned.zip`;
};

export const parseMacosArtifactIdentity = (
  input: MacosArtifactIdentityInput,
): MacosArtifactIdentity => {
  const rustc = parseRustcVersion(input.rustcVersionOutput);
  const expectedMachO = expectedMachOArchitecture(rustc.host);
  const lipoOutput = input.lipoArchsOutput ?? input.lipoOutput;
  if (lipoOutput === undefined) throw new Error('lipo -archs output is missing');
  let observedMachO: MachOArchitecture;
  try {
    observedMachO = parseLipoArchs(lipoOutput);
  } catch (error: unknown) {
    const observed = lipoOutput.trim() === '' ? '<empty>' : lipoOutput.trim();
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(`Mach-O architecture mismatch: expected ${expectedMachO}; observed ${observed}; ${reason}`);
  }
  if (observedMachO !== expectedMachO) {
    throw new Error(`Mach-O architecture ${observedMachO} does not match expected ${expectedMachO}`);
  }
  const archiveBasename = buildMacosArchiveBasename(input.packageVersion, rustc.host);
  if (input.archiveBasename !== undefined && input.archiveBasename !== archiveBasename) {
    throw new Error(`unexpected macOS distribution archive basename: ${input.archiveBasename}`);
  }
  return {
    schema: MACOS_DISTRIBUTION_SCHEMA,
    packageVersion: validateCargoPackageVersion(input.packageVersion),
    archiveBasename,
    checksumBasename: `${archiveBasename}.sha256`,
    rustRelease: rustc.release,
    rustHost: rustc.host,
    expectedMachOArchitecture: expectedMachO,
    observedMachOArchitecture: observedMachO,
  };
};

export type MacosSmokeLogInput = {
  identity: MacosArtifactIdentity;
  gitSha: string;
  bundleIdentifier: string;
  archiveSha256: string;
  archiveSize: number;
  executableSha256: string;
  executableSize: number;
  passRecords: readonly string[];
};

const assertSha256 = (label: string, value: string): void => {
  if (!/^[0-9a-f]{64}$/.test(value)) throw new Error(`${label} must be a lowercase SHA-256 digest`);
};

const assertSize = (label: string, value: number): void => {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`${label} must be a non-negative integer`);
};

export const renderMacosSmokeLog = (input: MacosSmokeLogInput): string => {
  const { identity } = input;
  if (identity.schema !== MACOS_DISTRIBUTION_SCHEMA) throw new Error('unexpected macOS smoke schema');
  if (input.gitSha.length !== 40 || !/^[0-9a-f]{40}$/.test(input.gitSha)) throw new Error('git SHA is invalid');
  if (input.bundleIdentifier !== MACOS_BUNDLE_IDENTIFIER) throw new Error('bundle identifier is invalid');
  assertSha256('archive SHA-256', input.archiveSha256);
  assertSha256('executable SHA-256', input.executableSha256);
  assertSize('archive size', input.archiveSize);
  assertSize('executable size', input.executableSize);
  if (input.passRecords.length === 0 || input.passRecords.at(-1) !== 'smoke-complete') {
    throw new Error('canonical smoke log requires a final success record');
  }
  const seen = new Set<string>();
  for (const record of input.passRecords) {
    if (!/^[a-z0-9][a-z0-9-]*$/.test(record) || seen.has(record)) {
      throw new Error(`invalid or duplicate smoke pass record: ${record}`);
    }
    seen.add(record);
  }
  return [
    `schema=${identity.schema}`,
    `git_sha=${input.gitSha}`,
    `cargo_package_version=${identity.packageVersion}`,
    `rust_release=${identity.rustRelease}`,
    `rust_host=${identity.rustHost}`,
    `expected_macho_architecture=${identity.expectedMachOArchitecture}`,
    `observed_macho_architecture=${identity.observedMachOArchitecture}`,
    `bundle_identifier=${input.bundleIdentifier}`,
    `archive_basename=${identity.archiveBasename}`,
    `checksum_basename=${identity.checksumBasename}`,
    `archive_sha256=${input.archiveSha256}`,
    `archive_size=${input.archiveSize}`,
    `executable_sha256=${input.executableSha256}`,
    `executable_size=${input.executableSize}`,
    ...input.passRecords.map((record) => `pass=${record}`),
    '',
  ].join('\n');
};

const readText = async (path: string): Promise<string> => Bun.file(path).text();

const main = async (): Promise<void> => {
  const args = Bun.argv.slice(2);
  if (args[0] === '--render-log') {
    if (args.length !== 9) throw new Error('usage: macos-artifact-identity.ts --render-log IDENTITY GIT_SHA BUNDLE_ID ARCHIVE_SHA ARCHIVE_SIZE EXECUTABLE_SHA EXECUTABLE_SIZE PASSES');
    const [, identityPath, gitSha, bundleIdentifier, archiveSha256, archiveSizeText, executableSha256, executableSizeText, passPath] = args;
    if ([identityPath, gitSha, bundleIdentifier, archiveSha256, archiveSizeText, executableSha256, executableSizeText, passPath].some((value) => value === undefined)) {
      throw new Error('macOS smoke log arguments must not be missing');
    }
    const identity = JSON.parse(await readText(identityPath)) as MacosArtifactIdentity;
    const archiveSize = Number(archiveSizeText);
    const executableSize = Number(executableSizeText);
    const passRecords = (await readText(passPath)).split(/\r?\n/).filter((record) => record !== '');
    process.stdout.write(renderMacosSmokeLog({
      identity,
      gitSha,
      bundleIdentifier,
      archiveSha256,
      archiveSize,
      executableSha256,
      executableSize,
      passRecords,
    }));
    return;
  }
  if (args.length !== 3 && args.length !== 4) {
    throw new Error('usage: macos-artifact-identity.ts RUSTC_OUTPUT LIPO_OUTPUT VERSION [ARCHIVE_BASENAME]');
  }
  const [rustcPath, lipoPath, packageVersion, archiveBasename] = args;
  if ([rustcPath, lipoPath, packageVersion].some((value) => value === undefined)) {
    throw new Error('macOS identity arguments must not be missing');
  }
  process.stdout.write(`${JSON.stringify(parseMacosArtifactIdentity({
    rustcVersionOutput: await readText(rustcPath),
    lipoArchsOutput: await readText(lipoPath),
    packageVersion,
    archiveBasename,
  }))}\n`);
};

if (import.meta.main) {
  await main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
