const requiredRange = "^20.19.0 || >=22.12.0";
const match = process.versions.node.match(/^(\d+)\.(\d+)\.(\d+)/);

if (!match) {
  console.error(`Unsupported Node version: ${process.versions.node}. Required: ${requiredRange}.`);
  process.exit(1);
}

const [, majorText, minorText, patchText] = match;
const version = {
  major: Number(majorText),
  minor: Number(minorText),
  patch: Number(patchText),
};

const atLeast = (major, minor, patch) =>
  version.major > major ||
  (version.major === major &&
    (version.minor > minor || (version.minor === minor && version.patch >= patch)));

const supported =
  (version.major === 20 && atLeast(20, 19, 0)) || atLeast(22, 12, 0);

if (!supported) {
  console.error(
    `Node ${process.versions.node} is unsupported. Vite requires ${requiredRange}.`,
  );
  process.exit(1);
}

process.stdout.write(`Node ${process.versions.node} satisfies ${requiredRange}.\n`);
