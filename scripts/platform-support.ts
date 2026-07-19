#!/usr/bin/env bun

import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

export type SupportedTarget = {
  triple: string;
  os: string;
  architecture: string;
  runner: string;
};

type PlatformContract = {
  schema_version?: number;
  targets?: SupportedTarget[];
};

export const validatePlatformTargets = (contract: PlatformContract): SupportedTarget[] => {
  if (contract.schema_version !== 1) throw new Error('platform contract schema_version must be 1');
  const targets = contract.targets ?? [];
  const triples = new Set<string>();
  for (const target of targets) {
    if (!target.triple || !target.os || !target.architecture || !target.runner || triples.has(target.triple)) {
      throw new Error('platform contract targets must have unique non-empty fields');
    }
    triples.add(target.triple);
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
