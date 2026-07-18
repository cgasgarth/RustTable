const manifestPath = Bun.env.RUSTTABLE_PACKAGE_MANIFEST;
if (!manifestPath) {
  console.error("package manifest path is required");
  process.exit(2);
}

const manifest = await Bun.file(manifestPath).json();
if (manifest.packageManager !== "bun@1.3.14") {
  console.error("packageManager must be exactly bun@1.3.14");
  process.exit(1);
}

for (const section of ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"]) {
  if (manifest[section] !== undefined) {
    console.error(`${section} is prohibited until JavaScript dependencies are deliberately audited`);
    process.exit(1);
  }
}

const manifestDirectory = manifestPath.substring(0, manifestPath.lastIndexOf("/"));
for (const lockfile of ["bun.lock", "bun.lockb"]) {
  if (await Bun.file(`${manifestDirectory}/${lockfile}`).exists()) {
    console.error(`${lockfile} is prohibited while JavaScript dependencies are zero`);
    process.exit(1);
  }
}

console.log("javascript dependency policy: zero packages");
