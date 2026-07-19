#!/usr/bin/env bun

import { lstat, readFile, realpath } from 'node:fs/promises';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';

const EXPECTED_VERSION = '1.98';
const ROOT_SECTION = 'workspace.package';
const PACKAGE_SECTION = 'package';

type Assignment = {
  section: string;
  key: string;
  value: string;
  line: number;
};

type Toml = {
  assignments: Assignment[];
  errors: string[];
};

export type WorkspacePackage = {
  id: string;
  name: string;
  manifest_path: string;
  rust_version: unknown;
};

export type WorkspaceInput = {
  root: string;
  rootManifest: string;
  metadata: string;
  packageManifests: Record<string, string>;
};

export type CargoFailure = {
  exitCode: number;
  stderr: string;
};

const stripComment = (line: string): string => {
  let quoted = false;
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (character === '\\' && quoted) {
      escaped = !escaped;
      continue;
    }
    if (character === '"' && !escaped) quoted = !quoted;
    if (character === '#' && !quoted) return line.slice(0, index);
    escaped = false;
  }
  return line;
};

const parseToml = (source: string): Toml => {
  const assignments: Assignment[] = [];
  const errors: string[] = [];
  let section = '';

  for (const [index, rawLine] of source.split(/\r?\n/).entries()) {
    const line = stripComment(rawLine).trim();
    const lineNumber = index + 1;
    if (line === '') continue;

    const table = /^\[([^\[\]]+)\]$/.exec(line);
    if (table) {
      section = table[1]?.trim() ?? '';
      continue;
    }
    if (/^\[\[.*\]\]$/.test(line)) {
      section = '';
      continue;
    }

    const assignment = /^([A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*)\s*=\s*(.*)$/.exec(line);
    if (!assignment) {
      if (section === ROOT_SECTION || section === PACKAGE_SECTION) {
        errors.push(`line ${lineNumber}: malformed assignment`);
      }
      continue;
    }
    assignments.push({ section, key: assignment[1] ?? '', value: assignment[2]?.trim() ?? '', line: lineNumber });
  }

  return { assignments, errors };
};

const exactAssignments = (toml: Toml, section: string, key: string): Assignment[] =>
  toml.assignments.filter((assignment) => assignment.section === section && assignment.key === key);

const isExactStableVersion = (value: string): boolean => /^"[0-9]+\.[0-9]+"$/.test(value);

const isWithin = (root: string, candidate: string): boolean => {
  const child = relative(root, candidate);
  return child === '' || (!child.startsWith(`..${sep}`) && child !== '..' && !isAbsolute(child));
};

const expectedManifestPath = (root: string, name: string): string =>
  resolve(root, 'crates', name, 'Cargo.toml');

const parseMetadata = (source: string): { value: unknown; errors: string[] } => {
  try {
    return { value: JSON.parse(source) as unknown, errors: [] };
  } catch {
    return { value: null, errors: ['cargo metadata: malformed JSON'] };
  }
};

const recordOf = (value: unknown): Record<string, unknown> =>
  typeof value === 'object' && value !== null ? value as Record<string, unknown> : {};

const stringValue = (value: unknown): string => typeof value === 'string' ? value : '';

const metadataPackages = (value: unknown): WorkspacePackage[] => {
  const packages = recordOf(value).packages;
  if (!Array.isArray(packages)) return [];
  return packages.map((entry) => {
    const record = recordOf(entry);
    return {
      id: stringValue(record.id),
      name: stringValue(record.name),
      manifest_path: stringValue(record.manifest_path),
      rust_version: record.rust_version,
    };
  });
};

const packageName = (entry: WorkspacePackage): string => entry.name || entry.id || '<unnamed>';

const validateRootManifest = (source: string): { version: string | null; errors: string[] } => {
  const toml = parseToml(source);
  const errors = [...toml.errors.map((error) => `root Cargo.toml: ${error}`)];
  const fields = exactAssignments(toml, ROOT_SECTION, 'rust-version');
  if (fields.length === 0) {
    errors.push('root Cargo.toml: [workspace.package] rust-version is missing');
    return { version: null, errors };
  }
  if (fields.length > 1) {
    errors.push('root Cargo.toml: [workspace.package] rust-version is duplicated');
    return { version: null, errors };
  }
  const value = fields[0]?.value ?? '';
  if (!isExactStableVersion(value)) {
    errors.push('root Cargo.toml: [workspace.package] rust-version must be an exact stable major.minor release');
    return { version: null, errors };
  }
  const version = value.slice(1, -1);
  if (version !== EXPECTED_VERSION) {
    errors.push(`root Cargo.toml: rust-version must be ${EXPECTED_VERSION}, found ${version}`);
    return { version: null, errors };
  }
  return { version, errors };
};

const validatePackageManifest = (name: string, source: string): string[] => {
  const toml = parseToml(source);
  const errors = toml.errors.map((error) => `package ${name}: ${error}`);
  const inheritance = exactAssignments(toml, PACKAGE_SECTION, 'rust-version.workspace');
  const local = exactAssignments(toml, PACKAGE_SECTION, 'rust-version');

  if (local.length > 0) {
    errors.push(`package ${name}: package-local rust-version is forbidden`);
    if (local.length > 1) errors.push(`package ${name}: package-local rust-version is duplicated`);
  }
  if (inheritance.length === 0) {
    errors.push(`package ${name}: rust-version.workspace = true is missing`);
  } else if (inheritance.length > 1) {
    errors.push(`package ${name}: rust-version.workspace is duplicated`);
  } else if (inheritance[0]?.value !== 'true') {
    errors.push(`package ${name}: rust-version.workspace must be boolean true`);
  }
  return errors;
};

const validateMetadata = (
  root: string,
  source: string,
  expectedVersion: string | null,
  packageManifests: Record<string, string>,
): string[] => {
  const parsed = parseMetadata(source);
  if (parsed.errors.length > 0) return parsed.errors;
  const metadata = recordOf(parsed.value);
  const members = metadata.workspace_members;
  const packages = metadataPackages(parsed.value);
  const errors: string[] = [];
  if (!Array.isArray(members) || members.length === 0 || packages.length === 0) {
    errors.push('cargo metadata: workspace package set is empty');
    return errors;
  }

  const memberIds = members.map(stringValue);
  const memberCounts = new Map<string, number>();
  for (const id of memberIds) memberCounts.set(id, (memberCounts.get(id) ?? 0) + 1);
  for (const [id, count] of memberCounts) {
    if (!id) errors.push('cargo metadata: workspace member entry is malformed');
    if (count > 1) errors.push(`cargo metadata: workspace member ${id || '<empty>'} is duplicated`);
  }

  const packageCounts = new Map<string, number>();
  for (const entry of packages) packageCounts.set(entry.id, (packageCounts.get(entry.id) ?? 0) + 1);
  for (const [id, count] of packageCounts) {
    if (!id) errors.push('cargo metadata: package entry has no id');
    if (count > 1) errors.push(`cargo metadata: package entry ${id || '<empty>'} is duplicated`);
  }

  const entries = [...packages].sort((left, right) => packageName(left).localeCompare(packageName(right)));
  for (const entry of entries) {
    if (!memberCounts.has(entry.id)) {
      errors.push(`package ${packageName(entry)}: metadata package is not a workspace member`);
    }
  }

  const workspaceEntries = memberIds
    .filter((id, index, all) => all.indexOf(id) === index)
    .map((id) => packages.find((entry) => entry.id === id))
    .sort((left, right) => packageName(left ?? { id: '', name: '', manifest_path: '', rust_version: null })
      .localeCompare(packageName(right ?? { id: '', name: '', manifest_path: '', rust_version: null })));

  for (const entry of workspaceEntries) {
    if (!entry) {
      continue;
    }
    const name = packageName(entry);
    if (!entry.name || !entry.manifest_path) {
      errors.push(`package ${name}: metadata entry is malformed`);
      continue;
    }
    if (expectedVersion !== null && entry.rust_version !== expectedVersion) {
      errors.push(`package ${name}: metadata rust_version must be ${expectedVersion}, found ${String(entry.rust_version)}`);
    }
    const expectedPath = expectedManifestPath(root, entry.name);
    const observedPath = resolve(entry.manifest_path);
    if (!isWithin(root, observedPath) || observedPath !== expectedPath) {
      errors.push(`package ${name}: manifest must be crates/${entry.name}/Cargo.toml`);
      continue;
    }
    const manifest = packageManifests[observedPath];
    if (manifest === undefined) {
      errors.push(`package ${name}: manifest was not provided`);
      continue;
    }
    errors.push(...validatePackageManifest(name, manifest));
  }

  for (const id of memberIds) {
    if (!packages.some((entry) => entry.id === id)) errors.push(`cargo metadata: workspace package ${id} is missing`);
  }
  return errors;
};

export const validateWorkspace = (input: WorkspaceInput): string[] => {
  const root = resolve(input.root);
  const rootResult = validateRootManifest(input.rootManifest);
  const errors = [...rootResult.errors];
  errors.push(...validateMetadata(root, input.metadata, rootResult.version, input.packageManifests));
  return errors;
};

export const formatCargoFailure = ({ exitCode, stderr }: CargoFailure): string => {
  const detail = stderr.trim();
  return `cargo metadata failed with exit code ${exitCode}${detail === '' ? '' : `: ${detail}`}`;
};

const assertSafeManifestPath = async (root: string, candidate: string): Promise<void> => {
  const repositoryRoot = resolve(root);
  const target = resolve(candidate);
  if (!isWithin(repositoryRoot, target)) throw new Error(`manifest path escapes repository: ${candidate}`);
  const parts = relative(repositoryRoot, target).split(sep).filter(Boolean);
  let current = repositoryRoot;
  for (const part of parts) {
    current = join(current, part);
    const stat = await lstat(current);
    if (stat.isSymbolicLink()) throw new Error(`manifest path is a symlink: ${candidate}`);
  }
};

const runCargoMetadata = async (root: string): Promise<string> => {
  const child = Bun.spawn(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], {
    cwd: root,
    stderr: 'pipe',
    stdout: 'pipe',
  });
  const stdout = await new Response(child.stdout).text();
  const stderr = await new Response(child.stderr).text();
  const exitCode = await child.exited;
  if (exitCode !== 0) throw new Error(formatCargoFailure({ exitCode, stderr }));
  return stdout;
};

const loadPackageManifests = async (root: string, metadata: string): Promise<{ manifests: Record<string, string>; errors: string[] }> => {
  const parsed = parseMetadata(metadata);
  if (parsed.errors.length > 0) return { manifests: {}, errors: [] };
  const manifests: Record<string, string> = {};
  const errors: string[] = [];
  for (const entry of metadataPackages(parsed.value)) {
    if (entry.name === '' || entry.manifest_path === '') continue;
    const path = resolve(entry.manifest_path);
    if (path !== expectedManifestPath(root, entry.name)) continue;
    try {
      await assertSafeManifestPath(root, path);
      manifests[path] = await readFile(path, 'utf8');
    } catch (error) {
      errors.push(error instanceof Error ? error.message : String(error));
    }
  }
  return { manifests, errors };
};

export const checkRepository = async (repositoryRoot: string): Promise<string[]> => {
  const root = await realpath(resolve(repositoryRoot));
  const rootManifestPath = join(root, 'Cargo.toml');
  await assertSafeManifestPath(root, rootManifestPath);
  const rootManifest = await readFile(rootManifestPath, 'utf8');
  const metadata = await runCargoMetadata(root);
  const loaded = await loadPackageManifests(root, metadata);
  return [...loaded.errors, ...validateWorkspace({ root, rootManifest, metadata, packageManifests: loaded.manifests })];
};

const main = async (): Promise<void> => {
  try {
    const errors = await checkRepository(resolve(import.meta.dir, '..'));
    if (errors.length > 0) {
      for (const error of errors) console.error(`workspace rust-version: ${error}`);
      process.exitCode = 1;
      return;
    }
    console.log(`workspace rust-version: ${EXPECTED_VERSION} verified`);
  } catch (error) {
    console.error(`workspace rust-version: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
};

if (import.meta.main) await main();
