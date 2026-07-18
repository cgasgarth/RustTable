#!/usr/bin/env bun

export type WorkspaceDependency = {
  name: string;
  kind: string | null;
};

export type WorkspacePackage = {
  name: string;
  dependencies: WorkspaceDependency[];
};

export type WorkspaceLayoutInput = {
  files: Iterable<string>;
  packages: Iterable<WorkspacePackage>;
};

const REQUIRED_MEMBERS = [
  'rusttable-app',
  'rusttable-catalog',
  'rusttable-catalog-store',
  'rusttable-core',
  'rusttable-diagnostics',
  'rusttable-image',
  'rusttable-image-io',
  'rusttable-import',
  'rusttable-metadata',
  'rusttable-processing',
  'rusttable-render',
  'rusttable-ui',
];

const REQUIRED_PATHS = [
  'crates/rusttable-app/src/application/mod.rs',
  'crates/rusttable-app/src/composition/mod.rs',
  'crates/rusttable-app/src/library/loader.rs',
  'crates/rusttable-app/src/library/mod.rs',
  'crates/rusttable-app/src/lifecycle/mod.rs',
  'crates/rusttable-ui/src/input/mod.rs',
  'crates/rusttable-ui/src/navigation/mod.rs',
  'crates/rusttable-ui/src/presentation/mod.rs',
  'crates/rusttable-ui/src/library/mod.rs',
  'crates/rusttable-ui/src/state/mod.rs',
  'crates/rusttable-ui/src/theme/mod.rs',
  'crates/rusttable-ui/src/view/mod.rs',
  'crates/rusttable-ui/src/widgets/action_button.rs',
  'crates/rusttable-ui/src/widgets/mod.rs',
];

const APP_ROOT_FILES = new Set(['crates/rusttable-app/src/lib.rs', 'crates/rusttable-app/src/main.rs']);

const normalDependencies = (workspacePackage: WorkspacePackage): string[] =>
  workspacePackage.dependencies
    .filter((dependency) => dependency.kind === null)
    .map((dependency) => dependency.name)
    .sort();

export const validateWorkspaceLayout = (input: WorkspaceLayoutInput): string[] => {
  const files = [...new Set(input.files)].sort();
  const packages = [...input.packages].sort((left, right) => left.name.localeCompare(right.name));
  const packageNames = new Set(packages.map((workspacePackage) => workspacePackage.name));
  const errors: string[] = [];

  for (const member of REQUIRED_MEMBERS) {
    if (!packageNames.has(member)) errors.push(`workspace member is missing: ${member}`);
  }
  for (const path of REQUIRED_PATHS) {
    if (!files.includes(path)) errors.push(`required subsystem owner is missing: ${path}`);
  }

  for (const file of files) {
    if (!file.startsWith('crates/rusttable-app/src/')) continue;
    const relativePath = file.slice('crates/rusttable-app/src/'.length);
    if (!relativePath.includes('/') && !APP_ROOT_FILES.has(file)) {
      errors.push(`rusttable-app/src must keep high-level modules in owned directories: ${file}`);
    }
  }

  const app = packages.find((workspacePackage) => workspacePackage.name === 'rusttable-app');
  const ui = packages.find((workspacePackage) => workspacePackage.name === 'rusttable-ui');
  if (app && !normalDependencies(app).includes('rusttable-ui')) {
    errors.push('rusttable-app must depend on rusttable-ui as its UI boundary');
  }
  if (ui) {
    for (const dependency of normalDependencies(ui)) {
      if (dependency !== 'rusttable-core' && dependency !== 'iced') {
        errors.push(`rusttable-ui has a forbidden normal dependency: ${dependency}`);
      }
    }
  }
  for (const workspacePackage of packages) {
    if (workspacePackage.name === 'rusttable-app') continue;
    for (const dependency of normalDependencies(workspacePackage)) {
      if (dependency === 'rusttable-app') {
        errors.push(`${workspacePackage.name} must not depend on composition root rusttable-app`);
      }
      if (dependency === 'rusttable-ui' && workspacePackage.name !== 'rusttable-ui') {
        errors.push(`${workspacePackage.name} must not depend on rusttable-ui`);
      }
    }
  }

  return errors;
};

type CargoMetadata = {
  packages?: Array<{
    id?: string;
    name?: string;
    manifest_path?: string;
    dependencies?: Array<{ name?: string; kind?: string | null }>;
  }>;
  workspace_members?: string[];
};

const workspacePackages = (metadata: CargoMetadata): WorkspacePackage[] => {
  const members = new Set(metadata.workspace_members ?? []);
  return (metadata.packages ?? [])
    .filter((workspacePackage) => typeof workspacePackage.id === 'string' && members.has(workspacePackage.id))
    .map((workspacePackage) => ({
      name: workspacePackage.name ?? '',
      dependencies: (workspacePackage.dependencies ?? []).map((dependency) => ({
        name: dependency.name ?? '',
        kind: dependency.kind ?? null,
      })),
    }));
};

const relativeFiles = async (root: string): Promise<string[]> => {
  const glob = new Bun.Glob('crates/**/*.rs');
  const files: string[] = [];
  for await (const file of glob.scan({ cwd: root, onlyFiles: true })) files.push(file);
  return files;
};

if (import.meta.main) {
  const root = new URL('..', import.meta.url).pathname;
  const result = Bun.spawnSync(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], {
    cwd: root,
    stderr: 'pipe',
    stdout: 'pipe',
  });
  if (!result.success) {
    console.error(`workspace layout: cargo metadata failed\n${result.stderr.toString().trim()}`);
    process.exit(1);
  }

  let metadata: CargoMetadata;
  try {
    metadata = JSON.parse(result.stdout.toString()) as CargoMetadata;
  } catch (error) {
    console.error(`workspace layout: cargo metadata returned invalid JSON: ${String(error)}`);
    process.exit(1);
  }

  const errors = validateWorkspaceLayout({
    files: await relativeFiles(root),
    packages: workspacePackages(metadata),
  });
  if (errors.length > 0) {
    console.error('workspace layout: FAIL');
    for (const error of errors) console.error(` - ${error}`);
    process.exit(1);
  }
  console.log('workspace layout: PASS');
}
