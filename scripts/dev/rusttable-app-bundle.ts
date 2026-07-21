import { chmod, copyFile, mkdir, readFile, readdir, rm, stat } from 'node:fs/promises';
import { join, resolve } from 'node:path';

export const RUSTTABLE_BUNDLE_IDENTIFIER = 'com.cgasgarth.rusttable';
export const RUSTTABLE_BUNDLE_NAME = 'RustTable';
export const RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER = 'com.cgasgarth.rusttable.latest';
export const RUSTTABLE_COMPUTER_USE_BUNDLE_NAME = 'rusttable - latest';
export const RUSTTABLE_ICON_FILE = 'RustTable.icns';
const DEFAULT_ICON_PATH = join(import.meta.dir, 'assets', RUSTTABLE_ICON_FILE);

export interface RustTableBundleIdentity {
  bundleIdentifier: string;
  bundleName: string;
  displayName: string;
}

export const RUSTTABLE_BUNDLE_IDENTITY: RustTableBundleIdentity = {
  bundleIdentifier: RUSTTABLE_BUNDLE_IDENTIFIER,
  bundleName: RUSTTABLE_BUNDLE_NAME,
  displayName: RUSTTABLE_BUNDLE_NAME,
};

export const RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY: RustTableBundleIdentity = {
  bundleIdentifier: RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTIFIER,
  bundleName: RUSTTABLE_COMPUTER_USE_BUNDLE_NAME,
  displayName: RUSTTABLE_COMPUTER_USE_BUNDLE_NAME,
};

const REQUIRED_KEYS = [
  'CFBundleDisplayName',
  'CFBundleExecutable',
  'CFBundleIconFile',
  'CFBundleIdentifier',
  'CFBundleName',
  'CFBundlePackageType',
  'CFBundleShortVersionString',
  'CFBundleVersion',
] as const;

type RequiredKey = (typeof REQUIRED_KEYS)[number];

export type BundleManifest = Record<RequiredKey, string>;

export interface BundleDocumentType {
  role: 'Viewer';
  contentTypes: readonly string[];
  extensions: readonly string[];
}

/** Mirrors rusttable-image's standard decoder registry plus the explicit catalog-open policy. */
export const RUSTTABLE_DOCUMENT_TYPES: readonly BundleDocumentType[] = [
  {
    role: 'Viewer',
    contentTypes: ['public.image'],
    extensions: ['jpg', 'jpeg', 'png', 'tif', 'tiff'],
  },
  {
    role: 'Viewer',
    contentTypes: ['com.cgasgarth.rusttable.catalog'],
    extensions: ['redb'],
  },
];

export interface MetadataCommandRequest {
  args: readonly string[];
  command: string;
  label: string;
}

export interface MetadataCommandResult {
  exitCode: number;
  stderr: string;
  stdout: string;
}

export type MetadataCommandRunner = (
  request: MetadataCommandRequest,
) => Promise<MetadataCommandResult>;

const expectedManifest = (version: string, identity: RustTableBundleIdentity): BundleManifest => ({
  CFBundleDisplayName: identity.displayName,
  CFBundleExecutable: RUSTTABLE_BUNDLE_NAME,
  CFBundleIconFile: RUSTTABLE_ICON_FILE,
  CFBundleIdentifier: identity.bundleIdentifier,
  CFBundleName: identity.bundleName,
  CFBundlePackageType: 'APPL',
  CFBundleShortVersionString: version,
  CFBundleVersion: version,
});

const versionPattern = /^(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){0,2}$/;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const xmlUnescape = (value: string): string =>
  value.replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&quot;', '"').replaceAll('&apos;', "'").replaceAll('&amp;', '&');

const xmlEscape = (value: string): string =>
  value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;').replaceAll("'", '&apos;');

const assertVersion = (version: unknown): string => {
  if (typeof version !== 'string' || !versionPattern.test(version) || version.length > 18) {
    throw new Error('rusttable-app version cannot be represented by the macOS bundle version fields.');
  }
  return version;
};

export const parseCargoMetadataVersion = (metadata: string, expectedManifestPath?: string): string => {
  let parsed: unknown;
  try {
    parsed = JSON.parse(metadata) as unknown;
  } catch {
    throw new Error('cargo metadata returned malformed JSON.');
  }
  if (!isRecord(parsed) || !Array.isArray(parsed.packages)) {
    throw new Error('cargo metadata is missing its packages array.');
  }
  const candidates = parsed.packages.filter((candidate): candidate is Record<string, unknown> => {
    if (!isRecord(candidate) || candidate.name !== 'rusttable-app') return false;
    if (typeof expectedManifestPath !== 'string') return true;
    return candidate.manifest_path === expectedManifestPath;
  });
  if (candidates.length === 0) throw new Error('cargo metadata has no exact rusttable-app package.');
  if (candidates.length !== 1) throw new Error('cargo metadata has duplicate rusttable-app packages.');
  return assertVersion(candidates[0]?.version);
};

export const resolveRustTableVersion = async (
  root: string,
  run: MetadataCommandRunner,
): Promise<string> => {
  const result = await run({
    args: ['metadata', '--locked', '--no-deps', '--format-version', '1'],
    command: 'cargo',
    label: 'read locked RustTable package metadata',
  });
  return parseCargoMetadataVersion(result.stdout, resolve(root, 'crates/rusttable-app/Cargo.toml'));
};

export const renderBundlePlist = (manifest: BundleManifest): string => `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
${REQUIRED_KEYS.map((key) => `<key>${key}</key><string>${xmlEscape(manifest[key])}</string>`).join('\n')}
<key>CFBundleDocumentTypes</key><array>
${RUSTTABLE_DOCUMENT_TYPES.map((documentType) => `<dict><key>CFBundleTypeRole</key><string>${documentType.role}</string><key>LSItemContentTypes</key><array>${documentType.contentTypes.map((type) => `<string>${xmlEscape(type)}</string>`).join('')}</array><key>CFBundleTypeExtensions</key><array>${documentType.extensions.map((extension) => `<string>${xmlEscape(extension)}</string>`).join('')}</array></dict>`).join('\n')}
</array>
</dict></plist>
`;

const parseStringPairs = (plist: string): Map<string, string> => {
  const dictMatch = /<dict>([\s\S]*)<\/dict>/.exec(plist);
  if (dictMatch?.[1] === undefined) throw new Error('Bundle Info.plist must contain a dictionary.');
  const body = dictMatch[1];
  const pairs = [...body.matchAll(/<key>([^<]*)<\/key>\s*<string>([^<]*)<\/string>/g)];
  const values = new Map<string, string>();
  for (const pair of pairs) {
    const key = pair[1];
    const value = pair[2];
    if (key === undefined || value === undefined) throw new Error('Bundle Info.plist contains a malformed entry.');
    const decodedKey = xmlUnescape(key);
    if (decodedKey === 'CFBundleTypeRole') continue;
    if (values.has(decodedKey)) throw new Error(`Bundle Info.plist contains duplicate key ${decodedKey}.`);
    values.set(decodedKey, xmlUnescape(value));
  }
  return values;
};

const parseBundleDocumentTypes = (plist: string): BundleDocumentType[] => {
  const section = /<key>CFBundleDocumentTypes<\/key>\s*<array>([\s\S]*)<\/array>\s*<\/dict>/.exec(plist);
  if (section?.[1] === undefined) throw new Error('Bundle Info.plist is missing document declarations.');
  return [...section[1].matchAll(/<dict>([\s\S]*?)<\/dict>/g)].map((match) => {
    const body = match[1] ?? '';
    const role = /<key>CFBundleTypeRole<\/key>\s*<string>([^<]*)<\/string>/.exec(body)?.[1];
    const contentTypes = [...body.matchAll(/<key>LSItemContentTypes<\/key>\s*<array>([\s\S]*?)<\/array>/g)]
      .flatMap((entry) => [...(entry[1] ?? '').matchAll(/<string>([^<]*)<\/string>/g)].map((value) => xmlUnescape(value[1] ?? '')));
    const extensions = [...body.matchAll(/<key>CFBundleTypeExtensions<\/key>\s*<array>([\s\S]*?)<\/array>/g)]
      .flatMap((entry) => [...(entry[1] ?? '').matchAll(/<string>([^<]*)<\/string>/g)].map((value) => xmlUnescape(value[1] ?? '')));
    if (role !== 'Viewer' || contentTypes.length === 0 || extensions.length === 0) {
      throw new Error('Bundle Info.plist contains an invalid document declaration.');
    }
    return { role, contentTypes, extensions };
  });
};

export const parseBundleManifest = (
  plist: string,
  identity?: RustTableBundleIdentity,
): BundleManifest => {
  const values = parseStringPairs(plist);
  values.delete('CFBundleTypeRole');
  if (values.size !== REQUIRED_KEYS.length) {
    const unexpected = [...values.keys()].filter((key) => !(REQUIRED_KEYS as readonly string[]).includes(key));
    if (unexpected.length > 0) throw new Error(`Bundle Info.plist has unexpected key ${unexpected[0]}.`);
  }
  const manifest = Object.fromEntries(REQUIRED_KEYS.map((key) => [key, values.get(key)])) as BundleManifest;
  const knownIdentity = identity ?? [RUSTTABLE_BUNDLE_IDENTITY, RUSTTABLE_COMPUTER_USE_BUNDLE_IDENTITY].find(
    (candidate) => candidate.bundleIdentifier === manifest.CFBundleIdentifier,
  );
  if (knownIdentity === undefined) throw new Error('Bundle Info.plist has an unexpected bundle identifier.');
  const expected = expectedManifest(manifest.CFBundleShortVersionString, knownIdentity);
  for (const key of REQUIRED_KEYS) {
    if (manifest[key] !== expected[key]) throw new Error(`Bundle Info.plist has unexpected ${key}.`);
  }
  if (JSON.stringify(parseBundleDocumentTypes(plist)) !== JSON.stringify(RUSTTABLE_DOCUMENT_TYPES)) {
    throw new Error('Bundle Info.plist document declarations are not canonical.');
  }
  assertVersion(manifest.CFBundleShortVersionString);
  return manifest;
};

export const readBundleManifest = async (bundlePath: string): Promise<BundleManifest> =>
  parseBundleManifest(await readFile(join(bundlePath, 'Contents/Info.plist'), 'utf8'));

export const parseBundleIdentifier = (plist: string): string => {
  const match = /<key>CFBundleIdentifier<\/key>\s*<string>([^<]+)<\/string>/.exec(plist);
  if (match?.[1] === undefined) throw new Error('Bundle is missing CFBundleIdentifier.');
  return xmlUnescape(match[1]);
};

const expectedPayload = new Set([
  'Contents',
  'Contents/Info.plist',
  'Contents/MacOS',
  'Contents/MacOS/RustTable',
  'Contents/Resources',
  'Contents/Resources/LICENSE',
  `Contents/Resources/${RUSTTABLE_ICON_FILE}`,
]);

const listPayload = async (root: string, relative = ''): Promise<string[]> => {
  const directory = join(root, relative);
  const entries = await readdir(directory, { withFileTypes: true });
  const paths: string[] = [];
  for (const entry of entries) {
    const child = relative === '' ? entry.name : join(relative, entry.name);
    paths.push(child);
    if (entry.isDirectory()) paths.push(...await listPayload(root, child));
    if (entry.isSymbolicLink()) throw new Error(`Bundle contains a symbolic link: ${child}.`);
  }
  return paths;
};

const bytesEqual = (left: Uint8Array, right: Uint8Array): boolean =>
  left.length === right.length && left.every((value, index) => value === right[index]);

export const validateBundle = async (
  bundlePath: string,
  rootLicensePath?: string,
  identity?: RustTableBundleIdentity,
): Promise<BundleManifest> => {
  const manifest = await parseBundleManifest(
    await readFile(join(bundlePath, 'Contents/Info.plist'), 'utf8'),
    identity,
  );
  const actualPayload = new Set(await listPayload(bundlePath));
  if (actualPayload.size !== expectedPayload.size || [...expectedPayload].some((path) => !actualPayload.has(path))) {
    throw new Error('RustTable.app contains an unexpected or missing payload entry.');
  }
  const executable = await stat(join(bundlePath, 'Contents/MacOS/RustTable'));
  if (!executable.isFile() || (executable.mode & 0o111) === 0) {
    throw new Error('RustTable.app executable is missing or not executable.');
  }
  if (rootLicensePath !== undefined) {
    const [rootLicense, bundleLicense] = await Promise.all([
      readFile(rootLicensePath),
      readFile(join(bundlePath, 'Contents/Resources/LICENSE')),
    ]);
    if (!bytesEqual(rootLicense, bundleLicense)) throw new Error('RustTable.app LICENSE differs from root LICENSE.');
  }
  const icon = await readFile(join(bundlePath, 'Contents/Resources', RUSTTABLE_ICON_FILE));
  if (!bytesEqual(icon.subarray(0, 4), Uint8Array.from([0x69, 0x63, 0x6e, 0x73]))) {
    throw new Error('RustTable.app icon is not a valid ICNS file.');
  }
  return manifest;
};

export const createRustTableBundle = async ({
  appPath,
  executablePath,
  licensePath,
  iconPath = DEFAULT_ICON_PATH,
  version,
  identity = RUSTTABLE_BUNDLE_IDENTITY,
}: {
  appPath: string;
  executablePath: string;
  iconPath?: string;
  licensePath: string;
  version: string;
  identity?: RustTableBundleIdentity;
}): Promise<string> => {
  const manifest = expectedManifest(assertVersion(version), identity);
  await rm(appPath, { force: true, recursive: true });
  await mkdir(join(appPath, 'Contents/MacOS'), { recursive: true });
  await mkdir(join(appPath, 'Contents/Resources'), { recursive: true });
  await copyFile(executablePath, join(appPath, 'Contents/MacOS/RustTable'));
  await chmod(join(appPath, 'Contents/MacOS/RustTable'), 0o755);
  await copyFile(licensePath, join(appPath, 'Contents/Resources/LICENSE'));
  await copyFile(iconPath, join(appPath, 'Contents/Resources', RUSTTABLE_ICON_FILE));
  await Bun.write(join(appPath, 'Contents/Info.plist'), renderBundlePlist(manifest));
  return appPath;
};
