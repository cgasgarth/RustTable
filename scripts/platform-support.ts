#!/usr/bin/env bun

import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

export type SupportedTarget = {
  triple: string;
  os: string;
  architecture: string;
  minimum_os: string;
  libc: string;
  minimum_libc?: string;
  desktop: boolean;
  headless: boolean;
  window_systems: string[];
  backends: string[];
  cpu_fallback: string;
  runner: string;
  package_target: string;
  support_level: string;
};

type PlatformContract = {
  schema_version?: number;
  targets?: SupportedTarget[];
};

export const validatePlatformTargets = (contract: PlatformContract): SupportedTarget[] => {
  if (contract.schema_version !== 1) throw new Error('platform contract schema_version must be 1');
  const targets = contract.targets ?? [];
  const triples = new Set<string>();
  const runners = new Set<string>();
  const packageTargets = new Set<string>();
  for (const target of targets) {
    if (
      !target.triple ||
      !target.os ||
      !target.architecture ||
      !target.minimum_os ||
      !target.libc ||
      !target.runner ||
      !target.package_target ||
      !target.support_level ||
      !Array.isArray(target.window_systems) ||
      !Array.isArray(target.backends) ||
      !target.cpu_fallback ||
      triples.has(target.triple) ||
      packageTargets.has(target.package_target) ||
      runners.has(target.runner)
    ) {
      throw new Error('platform contract targets must have unique non-empty fields');
    }
    if (!target.desktop && !target.headless) throw new Error('platform target must allow desktop or headless mode');
    if (target.desktop && target.os === 'linux' && target.window_systems.length === 0) {
      throw new Error('Linux desktop target must declare a window system');
    }
    if (target.backends.length === 0 || target.backends.length > 3) throw new Error('platform target backend order is invalid');
    triples.add(target.triple);
    packageTargets.add(target.package_target);
    runners.add(target.runner);
  }
  if (targets.length === 0) throw new Error('platform contract must declare at least one target');
  return [...targets];
};

export const loadPlatformTargets = async (root: string): Promise<SupportedTarget[]> => {
  const path = join(root, 'architecture', 'platform-support.toml');
  const parsed = (await Bun.file(path).text()).trim();
  return validatePlatformTargets(Bun.TOML.parse(parsed) as PlatformContract);
};

if (import.meta.main) {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const targets = await loadPlatformTargets(root);
  const args = process.argv.slice(2);
  const osIndex = args.indexOf('--target-os');
  const architectureIndex = args.indexOf('--target-architecture');
  const os = osIndex >= 0 ? args[osIndex + 1] : undefined;
  const architecture = architectureIndex >= 0 ? args[architectureIndex + 1] : undefined;
  const selected = os
    ? targets.filter((target) => target.os === os && (!architecture || target.architecture === architecture))
    : targets;
  if (selected.length === 0) throw new Error('requested platform is not supported');
  if (args.includes('--json')) process.stdout.write(`${JSON.stringify(selected)}\n`);
  else process.stdout.write(`${selected.map((target) => target.triple).join('\n')}\n`);
}
