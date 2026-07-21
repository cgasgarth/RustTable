import { describe, expect, test } from 'bun:test';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import {
  createRustTableBundle,
  parseBundleManifest,
  parseCargoMetadataVersion,
  renderBundlePlist,
  RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY,
  RUSTTABLE_ICON_FILE,
  validateBundle,
} from './rusttable-app-bundle';

const manifestPath = resolve('/repo/crates/rusttable-app/Cargo.toml');
const metadata = (packages: unknown[]): string => JSON.stringify({ packages });
const packageRecord = (version: unknown = '0.1.0', path = manifestPath): Record<string, unknown> => ({
  name: 'rusttable-app',
  version,
  manifest_path: path,
});

describe('RustTable bundle metadata and manifest contracts', () => {
  test('selects exactly the workspace app package and version', () => {
    expect(parseCargoMetadataVersion(metadata([{ name: 'rusttable-core', version: '0.1.0' }, packageRecord()]), manifestPath)).toBe('0.1.0');
  });

  test('rejects malformed, missing, duplicate, misplaced, non-string, and unrepresentable metadata', () => {
    expect(() => parseCargoMetadataVersion('{')).toThrow('malformed JSON');
    expect(() => parseCargoMetadataVersion(JSON.stringify({}))).toThrow('packages array');
    expect(() => parseCargoMetadataVersion(metadata([]), manifestPath)).toThrow('no exact');
    expect(() => parseCargoMetadataVersion(metadata([packageRecord(), packageRecord()]), manifestPath)).toThrow('duplicate');
    expect(() => parseCargoMetadataVersion(metadata([packageRecord('0.1.0', '/repo/other/Cargo.toml')]), manifestPath)).toThrow('no exact');
    expect(() => parseCargoMetadataVersion(metadata([packageRecord(42)]), manifestPath)).toThrow('represented');
    expect(() => parseCargoMetadataVersion(metadata([packageRecord('0.1.0-alpha.1')]), manifestPath)).toThrow('represented');
  });

  test('renders and round-trips the exact required manifest', () => {
    const plist = renderBundlePlist(parseBundleManifest(renderBundlePlist({
      CFBundleDisplayName: 'RustTable',
      CFBundleExecutable: 'RustTable',
      CFBundleIconFile: RUSTTABLE_ICON_FILE,
      CFBundleIdentifier: 'com.cgasgarth.rusttable',
      CFBundleName: 'RustTable',
      CFBundlePackageType: 'APPL',
      CFBundleShortVersionString: '0.1.0',
      CFBundleVersion: '0.1.0',
    })));
    expect(plist).toContain('<key>CFBundleIdentifier</key><string>com.cgasgarth.rusttable</string>');
    expect(plist).toContain('<key>CFBundleDocumentTypes</key><array>');
    expect(plist).toContain('<string>public.image</string>');
    expect(plist).toContain('<string>com.cgasgarth.rusttable.catalog</string>');
    expect(parseBundleManifest(plist)).toEqual({
      CFBundleDisplayName: 'RustTable',
      CFBundleExecutable: 'RustTable',
      CFBundleIconFile: RUSTTABLE_ICON_FILE,
      CFBundleIdentifier: 'com.cgasgarth.rusttable',
      CFBundleName: 'RustTable',
      CFBundlePackageType: 'APPL',
      CFBundleShortVersionString: '0.1.0',
      CFBundleVersion: '0.1.0',
    });
  });

  test('renders the dedicated Computer Use identity exactly', () => {
    const plist = renderBundlePlist(parseBundleManifest(renderBundlePlist({
      CFBundleDisplayName: 'rusttable - latest',
      CFBundleExecutable: 'RustTable',
      CFBundleIconFile: RUSTTABLE_ICON_FILE,
      CFBundleIdentifier: 'com.cgasgarth.rusttable.latest',
      CFBundleName: 'rusttable - latest',
      CFBundlePackageType: 'APPL',
      CFBundleShortVersionString: '0.1.0',
      CFBundleVersion: '0.1.0',
    }), RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY));
    expect(plist).toContain('<key>CFBundleDisplayName</key><string>rusttable - latest</string>');
    expect(plist).toContain('<key>CFBundleIdentifier</key><string>com.cgasgarth.rusttable.latest</string>');
  });

  test('rejects every missing, duplicate, and mismatched required field', () => {
    const manifest = {
      CFBundleDisplayName: 'RustTable',
      CFBundleExecutable: 'RustTable',
      CFBundleIconFile: RUSTTABLE_ICON_FILE,
      CFBundleIdentifier: 'com.cgasgarth.rusttable',
      CFBundleName: 'RustTable',
      CFBundlePackageType: 'APPL',
      CFBundleShortVersionString: '0.1.0',
      CFBundleVersion: '0.1.0',
    } as const;
    for (const key of Object.keys(manifest) as (keyof typeof manifest)[]) {
      const entry = `<key>${key}</key><string>${manifest[key]}</string>`;
      expect(() => parseBundleManifest(renderBundlePlist(manifest).replace(entry, ''))).toThrow();
      expect(() => parseBundleManifest(renderBundlePlist(manifest).replace('</dict>', `${entry}</dict>`))).toThrow('duplicate');
      expect(() => parseBundleManifest(renderBundlePlist(manifest).replace(entry, `<key>${key}</key><string>wrong</string>`))).toThrow('unexpected');
    }
  });

  test('validates exact payload, executable mode, and byte-identical license', async () => {
    const root = await mkdtemp(join(tmpdir(), 'rusttable-bundle-'));
    try {
      const executable = join(root, 'rusttable-app');
      const license = join(root, 'LICENSE');
      const appPath = join(root, 'RustTable.app');
      await writeFile(executable, '#!/bin/sh\nprintf RustTable');
      await writeFile(license, 'GPL-3.0-or-later\n');
      await createRustTableBundle({ appPath, executablePath: executable, licensePath: license, version: '0.1.0' });
      await validateBundle(appPath, license);
      expect((await readdir(join(appPath, 'Contents/Resources'))).sort()).toEqual(['LICENSE', RUSTTABLE_ICON_FILE]);
      await writeFile(join(appPath, 'Contents/Resources/LICENSE'), 'corrupt');
      await expect(validateBundle(appPath, license)).rejects.toThrow('LICENSE differs');
      await writeFile(join(appPath, 'Contents/Resources/LICENSE'), await readFile(license));
      await writeFile(join(appPath, 'Contents/Info.plist'), renderBundlePlist({
        CFBundleDisplayName: 'Other',
        CFBundleExecutable: 'RustTable',
        CFBundleIconFile: RUSTTABLE_ICON_FILE,
        CFBundleIdentifier: 'com.cgasgarth.rusttable',
        CFBundleName: 'RustTable',
        CFBundlePackageType: 'APPL',
        CFBundleShortVersionString: '0.1.0',
        CFBundleVersion: '0.1.0',
      }));
      await expect(validateBundle(appPath, license)).rejects.toThrow('unexpected CFBundleDisplayName');
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
