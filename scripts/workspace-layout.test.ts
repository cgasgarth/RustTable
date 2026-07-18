import { describe, expect, test } from 'bun:test';

import { validateWorkspaceLayout, type WorkspacePackage } from './workspace-layout';

const packages: WorkspacePackage[] = [
  { name: 'rusttable-app', dependencies: [{ name: 'rusttable-ui', kind: null }] },
  { name: 'rusttable-catalog', dependencies: [] },
  { name: 'rusttable-catalog-store', dependencies: [] },
  { name: 'rusttable-core', dependencies: [] },
  { name: 'rusttable-diagnostics', dependencies: [] },
  { name: 'rusttable-image', dependencies: [] },
  { name: 'rusttable-image-io', dependencies: [] },
  { name: 'rusttable-import', dependencies: [] },
  { name: 'rusttable-metadata', dependencies: [] },
  { name: 'rusttable-processing', dependencies: [] },
  { name: 'rusttable-render', dependencies: [] },
  { name: 'rusttable-ui', dependencies: [{ name: 'rusttable-core', kind: null }] },
];

const requiredFiles = [
  'crates/rusttable-app/src/application/mod.rs',
  'crates/rusttable-app/src/composition/mod.rs',
  'crates/rusttable-app/src/library/loader.rs',
  'crates/rusttable-app/src/library/mod.rs',
  'crates/rusttable-app/src/lifecycle/mod.rs',
  'crates/rusttable-app/src/lib.rs',
  'crates/rusttable-app/src/main.rs',
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

describe('workspace layout contract', () => {
  test('accepts the complete subsystem layout and is order-independent', () => {
    const forward = validateWorkspaceLayout({ files: requiredFiles, packages });
    const reverse = validateWorkspaceLayout({ files: [...requiredFiles].reverse(), packages: [...packages].reverse() });

    expect(forward).toEqual([]);
    expect(reverse).toEqual(forward);
  });

  test('reports missing owners and a return to the flat app source directory', () => {
    const errors = validateWorkspaceLayout({
      files: ['crates/rusttable-app/src/app.rs', 'crates/rusttable-app/src/lib.rs'],
      packages,
    });

    expect(errors).toContain('required subsystem owner is missing: crates/rusttable-ui/src/view/mod.rs');
    expect(errors).toContain(
      'rusttable-app/src must keep high-level modules in owned directories: crates/rusttable-app/src/app.rs',
    );
  });

  test('rejects forbidden dependency edges', () => {
    const errors = validateWorkspaceLayout({
      files: requiredFiles,
      packages: packages.map((workspacePackage) =>
        workspacePackage.name === 'rusttable-catalog'
          ? { ...workspacePackage, dependencies: [{ name: 'rusttable-ui', kind: null }] }
          : workspacePackage,
      ),
    });

    expect(errors).toContain('rusttable-catalog must not depend on rusttable-ui');
  });
});
