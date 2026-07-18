export const LINUX_DISTRIBUTION_SCHEMA = 'RUSTTABLE_LINUX_DISTRIBUTION_V1' as const;
export const LINUX_HOST = 'x86_64-unknown-linux-gnu' as const;

export type LinuxArtifactIdentity = {
  schema: typeof LINUX_DISTRIBUTION_SCHEMA;
  packageVersion: string;
  archiveBasename: string;
  rustRelease: string;
  rustHost: typeof LINUX_HOST;
  elfClass: 'ELF64';
  elfData: "2's complement, little endian";
  elfMachine: 'Advanced Micro Devices X86-64';
  elfType: 'DYN' | 'EXEC';
  interpreter: string;
  needed: string[];
};

export type LinuxArtifactIdentityInput = {
  rustcVersionOutput: string;
  elfHeaderOutput: string;
  elfProgramHeadersOutput: string;
  elfDynamicOutput: string;
  lddOutput: string;
  lddStatus: number;
  packageVersion: string;
  archiveBasename: string;
};

const field = (output: string, name: string): string => {
  const values = output
    .split(/\r?\n/)
    .flatMap((line) => {
      const match = new RegExp(`^\\s*${name}:\\s*(.*?)\\s*$`).exec(line);
      return match?.[1] === undefined ? [] : [match[1]];
    });
  if (values.length !== 1 || values[0] === undefined || values[0] === '') {
    throw new Error(`ELF header must contain exactly one ${name} field`);
  }
  return values[0];
};

export const parseRustcVersion = (output: string): { release: string; host: typeof LINUX_HOST } => {
  const release = field(output, 'release');
  const host = field(output, 'host');
  if (host !== LINUX_HOST) throw new Error(`unexpected rustc host: ${host}`);
  return { release, host };
};

export const parseElfHeader = (
  output: string,
): Pick<LinuxArtifactIdentity, 'elfClass' | 'elfData' | 'elfMachine' | 'elfType'> => {
  if (!/^\s*ELF Header:\s*$/m.test(output)) throw new Error('readelf header is not ELF input');
  const magic = /^\s*Magic:\s*(.*?)\s*$/m.exec(output)?.[1];
  if (magic === undefined || !/^7f 45 4c 46(?:\s|$)/i.test(magic)) throw new Error('readelf header has invalid ELF magic');
  const elfClass = field(output, 'Class');
  const elfData = field(output, 'Data');
  const elfMachine = field(output, 'Machine');
  const typeField = field(output, 'Type');
  if (elfClass !== 'ELF64') throw new Error(`unexpected ELF class: ${elfClass}`);
  if (elfData !== "2's complement, little endian") throw new Error(`unexpected ELF data: ${elfData}`);
  if (elfMachine !== 'Advanced Micro Devices X86-64') throw new Error(`unexpected ELF machine: ${elfMachine}`);
  const elfType = typeField.split(/\s+/, 1)[0];
  if (elfType !== 'DYN' && elfType !== 'EXEC') throw new Error(`unexpected ELF type: ${typeField}`);
  return { elfClass, elfData, elfMachine, elfType };
};

export const parseProgramInterpreter = (output: string): string => {
  const matches = [...output.matchAll(/Requesting program interpreter:\s*([^\]\r\n]+)\]/g)].map((match) => match[1]?.trim() ?? '');
  if (matches.length !== 1 || matches[0] === undefined || matches[0] === '') {
    throw new Error('readelf program headers must contain exactly one interpreter');
  }
  const interpreter = matches[0];
  if (!interpreter.startsWith('/') || interpreter.split('/').includes('..') || /\s/.test(interpreter)) {
    throw new Error(`program interpreter is not a safe absolute path: ${interpreter}`);
  }
  return interpreter;
};

export const parseNeededLibraries = (output: string): string[] => {
  const needed: string[] = [];
  for (const line of output.split(/\r?\n/)) {
    if (!line.includes('(NEEDED)')) continue;
    const match = /\(NEEDED\).*?\[([^\]]*)\]/.exec(line);
    const name = match?.[1]?.trim();
    if (name === undefined || name === '' || name.includes('/') || /\s/.test(name)) {
      throw new Error(`malformed NEEDED library: ${line.trim()}`);
    }
    if (needed.includes(name)) throw new Error(`duplicate NEEDED library: ${name}`);
    needed.push(name);
  }
  return needed.sort();
};

export const validateLdd = (output: string, status: number): void => {
  if (status !== 0) throw new Error(`ldd failed with status ${status}`);
  if (/not found/i.test(output)) throw new Error('ldd reported a missing library');
};

const validateVersion = (version: string): void => {
  if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`invalid Cargo package version: ${version}`);
  }
};

export const parseLinuxArtifactIdentity = (input: LinuxArtifactIdentityInput): LinuxArtifactIdentity => {
  validateVersion(input.packageVersion);
  const expectedArchive = `RustTable-${input.packageVersion}-x86_64-unknown-linux-gnu-unsigned.tar.gz`;
  if (input.archiveBasename !== expectedArchive || input.archiveBasename.includes('/')) {
    throw new Error(`unexpected Linux distribution archive basename: ${input.archiveBasename}`);
  }
  const rustc = parseRustcVersion(input.rustcVersionOutput);
  const elf = parseElfHeader(input.elfHeaderOutput);
  const interpreter = parseProgramInterpreter(input.elfProgramHeadersOutput);
  const needed = parseNeededLibraries(input.elfDynamicOutput);
  validateLdd(input.lddOutput, input.lddStatus);
  return {
    schema: LINUX_DISTRIBUTION_SCHEMA,
    packageVersion: input.packageVersion,
    archiveBasename: input.archiveBasename,
    rustRelease: rustc.release,
    rustHost: rustc.host,
    elfClass: elf.elfClass,
    elfData: elf.elfData,
    elfMachine: elf.elfMachine,
    elfType: elf.elfType,
    interpreter,
    needed,
  };
};

const main = async (): Promise<void> => {
  const args = Bun.argv.slice(2);
  if (args.length !== 8) throw new Error('usage: linux-artifact-identity.ts RUSTC ELF_HEADER ELF_PROGRAM ELF_DYNAMIC LDD LDD_STATUS VERSION ARCHIVE');
  const [rustcPath, elfHeaderPath, elfProgramPath, elfDynamicPath, lddPath, lddStatusText, packageVersion, archiveBasename] = args;
  if ([rustcPath, elfHeaderPath, elfProgramPath, elfDynamicPath, lddPath, lddStatusText, packageVersion, archiveBasename].some((value) => value === undefined)) {
    throw new Error('identity arguments must not be missing');
  }
  const lddStatus = Number(lddStatusText);
  if (!Number.isInteger(lddStatus)) throw new Error(`invalid ldd status: ${lddStatusText}`);
  const input: LinuxArtifactIdentityInput = {
    rustcVersionOutput: await Bun.file(rustcPath).text(),
    elfHeaderOutput: await Bun.file(elfHeaderPath).text(),
    elfProgramHeadersOutput: await Bun.file(elfProgramPath).text(),
    elfDynamicOutput: await Bun.file(elfDynamicPath).text(),
    lddOutput: await Bun.file(lddPath).text(),
    lddStatus,
    packageVersion,
    archiveBasename,
  };
  process.stdout.write(`${JSON.stringify(parseLinuxArtifactIdentity(input))}\n`);
};

if (import.meta.main) {
  await main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
