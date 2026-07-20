import { describe, expect, test } from 'bun:test';
import { execFileSync } from 'node:child_process';
import {
  buildMacosArchiveBasename,
  expectedMachOArchitecture,
  parseLipoArchs,
  parseMacosArtifactIdentity,
  parseRustcVersion,
  renderMacosSmokeLog,
} from './macos-artifact-identity';

const rustRelease = /^release: (\S+)$/m.exec(
  execFileSync('rustc', ['-vV'], { encoding: 'utf8' }),
)?.[1];
if (!rustRelease) throw new Error('selected rustc release is missing');
const rustc = (host: string, release = rustRelease): string =>
  `rustc ${release} (fixture)\nbinary: rustc\ncommit-hash: fixture\ncommit-date: fixture\nhost: ${host}\nrelease: ${release}\nLLVM version: 20.1.0\n`;

const validInput = {
  rustcVersionOutput: rustc('aarch64-apple-darwin'),
  lipoArchsOutput: 'arm64\n',
  packageVersion: '0.1.0',
};

describe('macOS artifact identity', () => {
  test.each([
    ['aarch64-apple-darwin', 'arm64'],
    ['x86_64-apple-darwin', 'x86_64'],
  ])('maps %s to one expected Mach-O architecture', (host, architecture) => {
    expect(expectedMachOArchitecture(host)).toBe(architecture);
    expect(parseRustcVersion(rustc(host))).toEqual({ release: rustRelease, host });
  });

  test('constructs the target-qualified archive and canonical identity', () => {
    const identity = parseMacosArtifactIdentity(validInput);
    expect(identity).toEqual({
      schema: 'RUSTTABLE_MACOS_DISTRIBUTION_V2',
      packageVersion: '0.1.0',
      archiveBasename: 'RustTable-0.1.0-aarch64-apple-darwin-unsigned.zip',
      checksumBasename: 'RustTable-0.1.0-aarch64-apple-darwin-unsigned.zip.sha256',
      rustRelease,
      rustHost: 'aarch64-apple-darwin',
      expectedMachOArchitecture: 'arm64',
      observedMachOArchitecture: 'arm64',
    });
    expect(buildMacosArchiveBasename('0.1.0', 'x86_64-apple-darwin')).toBe(
      'RustTable-0.1.0-x86_64-apple-darwin-unsigned.zip',
    );
  });

  test.each([
    ['missing release', rustc('aarch64-apple-darwin').replace(`release: ${rustRelease}\n`, '')],
    ['duplicate release', `${rustc('aarch64-apple-darwin')}release: ${rustRelease}\n`],
    ['empty release', rustc('aarch64-apple-darwin').replace(`release: ${rustRelease}`, 'release:')],
    ['nightly release', rustc('aarch64-apple-darwin').replace(`release: ${rustRelease}`, `release: ${rustRelease}-nightly`)],
    ['missing host', rustc('aarch64-apple-darwin').replace('host: aarch64-apple-darwin\n', '')],
    ['duplicate host', `${rustc('aarch64-apple-darwin')}host: aarch64-apple-darwin\n`],
    ['unsupported host', rustc('x86_64-unknown-linux-gnu')],
  ])('%s rustc output is rejected', (_label, output) => {
    expect(() => parseRustcVersion(output)).toThrow();
  });

  test.each([
    ['empty', ''],
    ['duplicate', 'arm64 arm64'],
    ['unknown', 'arm64 armv7'],
    ['malformed token', 'arm64?'],
    ['universal', 'arm64 x86_64'],
  ])('%s lipo output is rejected', (_label, output) => {
    expect(() => parseLipoArchs(output)).toThrow();
  });

  test.each([
    ['wrong architecture', { ...validInput, lipoArchsOutput: 'x86_64' }],
    ['universal architecture', { ...validInput, lipoArchsOutput: 'arm64 x86_64' }],
    ['unknown architecture', { ...validInput, lipoArchsOutput: 'armv7' }],
    ['wrong archive basename', { ...validInput, archiveBasename: 'RustTable-0.1.0-macos-unsigned.zip' }],
    ['invalid Cargo version', { ...validInput, packageVersion: '0.1.0+unsafe/path' }],
  ])('%s identity is rejected', (_label, input) => {
    expect(() => parseMacosArtifactIdentity(input)).toThrow();
  });

  test('names expected and observed identities for malformed lipo output', () => {
    expect(() => parseMacosArtifactIdentity({ ...validInput, lipoArchsOutput: 'arm64 x86_64' })).toThrow(
      'expected arm64; observed arm64 x86_64',
    );
  });

  test('renders the canonical log in fixed order and requires final success', () => {
    const identity = parseMacosArtifactIdentity(validInput);
    const log = renderMacosSmokeLog({
      identity,
      gitSha: 'a'.repeat(40),
      bundleIdentifier: 'com.cgasgarth.rusttable',
      archiveSha256: 'b'.repeat(64),
      archiveSize: 123,
      executableSha256: 'c'.repeat(64),
      executableSize: 456,
      passRecords: ['bundle-build', 'staged-artifact-identity', 'smoke-complete'],
    });
    expect(log).toBe([
      'schema=RUSTTABLE_MACOS_DISTRIBUTION_V2',
      'git_sha=' + 'a'.repeat(40),
      'cargo_package_version=0.1.0',
      `rust_release=${rustRelease}`,
      'rust_host=aarch64-apple-darwin',
      'expected_macho_architecture=arm64',
      'observed_macho_architecture=arm64',
      'bundle_identifier=com.cgasgarth.rusttable',
      'archive_basename=RustTable-0.1.0-aarch64-apple-darwin-unsigned.zip',
      'checksum_basename=RustTable-0.1.0-aarch64-apple-darwin-unsigned.zip.sha256',
      'archive_sha256=' + 'b'.repeat(64),
      'archive_size=123',
      'executable_sha256=' + 'c'.repeat(64),
      'executable_size=456',
      'pass=bundle-build',
      'pass=staged-artifact-identity',
      'pass=smoke-complete',
    ].join('\n') + '\n');
    expect(() => renderMacosSmokeLog({
      identity,
      gitSha: 'a'.repeat(40),
      bundleIdentifier: 'com.cgasgarth.rusttable',
      archiveSha256: 'b'.repeat(64),
      archiveSize: 123,
      executableSha256: 'c'.repeat(64),
      executableSize: 456,
      passRecords: ['bundle-build'],
    })).toThrow('final success');
  });
});
