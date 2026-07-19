import { describe, expect, test } from 'bun:test';
import {
  parseElfHeader,
  parseLinuxArtifactIdentity,
  parseNeededLibraries,
  parseProgramInterpreter,
  parseRustcVersion,
  validateLdd,
} from './linux-artifact-identity';

const rustc = 'rustc 1.98.0-beta.4 (fixture)\nrelease: 1.98.0-beta.4\ncommit-hash: fixture\nhost: x86_64-unknown-linux-gnu\n';
const elfHeader = `ELF Header:
  Magic:   7f 45 4c 46 02 01 01 00
  Class:                             ELF64
  Data:                              2's complement, little endian
  Version:                           1 (current)
  Type:                              DYN (Position-Independent Executable file)
  Machine:                           Advanced Micro Devices X86-64
`;
const elfProgram = '  [Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]\n';
const elfDynamic = ` 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x0000000000000001 (NEEDED)             Shared library: [libm.so.6]
`;
const ldd = 'linux-vdso.so.1 (0x00007fff)\nlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f)\n';
const archive = 'RustTable-0.1.0-x86_64-unknown-linux-gnu-unsigned.tar.gz';

describe('Linux artifact identity', () => {
  test('accepts the canonical native ELF identity and sorts dependencies', () => {
    expect(parseLinuxArtifactIdentity({
      rustcVersionOutput: rustc,
      elfHeaderOutput: elfHeader,
      elfProgramHeadersOutput: elfProgram,
      elfDynamicOutput: `${elfDynamic} 0x0000000000000001 (NEEDED)             Shared library: [libz.so.1]\n`,
      lddOutput: ldd,
      lddStatus: 0,
      packageVersion: '0.1.0',
      archiveBasename: archive,
    })).toMatchObject({
      schema: 'RUSTTABLE_LINUX_DISTRIBUTION_V1',
      rustHost: 'x86_64-unknown-linux-gnu',
      elfType: 'DYN',
      needed: ['libc.so.6', 'libm.so.6', 'libz.so.1'],
    });
  });

  test('parses each authoritative identity source strictly', () => {
    expect(parseRustcVersion(rustc)).toEqual({ release: '1.98.0-beta.4', host: 'x86_64-unknown-linux-gnu' });
    expect(parseElfHeader(elfHeader).elfClass).toBe('ELF64');
    expect(parseProgramInterpreter(elfProgram)).toBe('/lib64/ld-linux-x86-64.so.2');
    expect(parseNeededLibraries(elfDynamic)).toEqual(['libc.so.6', 'libm.so.6']);
    expect(() => validateLdd('libmissing.so => not found', 0)).toThrow('missing library');
  });

  test.each([
    ['opposite Rust host', () => parseRustcVersion(rustc.replaceAll('x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'))],
    ['duplicate Rust host', () => parseRustcVersion(`${rustc}host: x86_64-unknown-linux-gnu\n`)],
    ['non-ELF input', () => parseElfHeader('not an ELF header')],
    ['wrong class', () => parseElfHeader(elfHeader.replace('ELF64', 'ELF32'))],
    ['duplicate ELF field', () => parseElfHeader(`${elfHeader}  Class: ELF64\n`)],
    ['wrong machine', () => parseElfHeader(elfHeader.replace('Advanced Micro Devices X86-64', 'AArch64'))],
    ['missing interpreter', () => parseProgramInterpreter('Program Headers:\n')],
    ['multiple interpreters', () => parseProgramInterpreter(`${elfProgram}${elfProgram}`)],
    ['relative interpreter', () => parseProgramInterpreter(elfProgram.replace('/lib64/', 'lib64/'))],
    ['duplicate NEEDED name', () => parseNeededLibraries(`${elfDynamic} 0x1 (NEEDED) Shared library: [libc.so.6]\n`)],
    ['malformed NEEDED name', () => parseNeededLibraries('0x1 (NEEDED) Shared library: [/tmp/libbad.so]\n')],
    ['ldd failure', () => validateLdd('', 1)],
    ['invalid archive basename', () => parseLinuxArtifactIdentity({ rustcVersionOutput: rustc, elfHeaderOutput: elfHeader, elfProgramHeadersOutput: elfProgram, elfDynamicOutput: elfDynamic, lddOutput: ldd, lddStatus: 0, packageVersion: '0.1.0', archiveBasename: 'wrong.tar.gz' })],
  ])('%s is rejected', (_label, operation) => {
    expect(operation).toThrow();
  });
});
