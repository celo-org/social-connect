// Bundles the monitor functions into ./deploy for `firebase deploy`.
//
// Firebase uploads only the functions source directory and Cloud Build installs its dependencies
// from npm. Deploying apps/monitor directly would therefore run the published @celo/* SDK
// releases instead of this repository's workspace packages, and would ignore the root yarn.lock.
// Bundling compiles the workspace code and the dependency versions resolved by yarn.lock into a
// single file. Only packages that cannot be bundled are installed by Cloud Build.
import { build } from 'esbuild'
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const outDir = join(root, 'deploy')
const require = createRequire(join(root, 'package.json'))

// Kept out of the bundle:
// - firebase-functions / firebase-admin: the Firebase CLI and runtime load them from node_modules.
// - blind-threshold-bls / @celo/poprf: they read their .wasm file relative to __dirname.
const EXTERNAL = ['firebase-functions', 'firebase-admin', 'blind-threshold-bls', '@celo/poprf']
// Optional native addon that bunyan loads inside try/catch. Left out and not installed.
const OPTIONAL_NOT_INSTALLED = ['dtrace-provider']

// npm registry written into the deploy lockfile. Cloud Build cannot authenticate to a private
// mirror, so resolved URLs must point at the public registry.
const PUBLIC_REGISTRY = 'https://registry.npmjs.org/'

// Returns the dependency specifier for the version yarn installed, keeping npm aliases such as
// blind-threshold-bls -> npm:@celo/blind-threshold-bls@1.0.0-beta.
function installedSpecifier(name) {
  let dir = dirname(require.resolve(name))
  // Walk up to the package root; some packages point "main" into a subdirectory.
  for (;;) {
    if (dir.endsWith(join('node_modules', name))) {
      const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'))
      return pkg.name === name ? pkg.version : `npm:${pkg.name}@${pkg.version}`
    }
    const parent = dirname(dir)
    if (parent === dir) {
      throw new Error(`Could not find package.json for ${name}`)
    }
    dir = parent
  }
}

rmSync(outDir, { recursive: true, force: true })
mkdirSync(outDir)

const result = await build({
  entryPoints: [join(root, 'src/index.ts')],
  bundle: true,
  platform: 'node',
  target: 'node24',
  format: 'cjs',
  outfile: join(outDir, 'index.js'),
  metafile: true,
  external: [...EXTERNAL, ...OPTIONAL_NOT_INSTALLED],
  logLevel: 'warning',
})
writeFileSync(join(root, 'meta.json'), JSON.stringify(result.metafile))

const monitorPkg = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
const deployPkg = {
  name: monitorPkg.name,
  version: monitorPkg.version,
  private: true,
  main: 'index.js',
  engines: monitorPkg.engines,
  dependencies: Object.fromEntries(EXTERNAL.map((name) => [name, installedSpecifier(name)])),
}
writeFileSync(join(outDir, 'package.json'), JSON.stringify(deployPkg, null, 2) + '\n')

// Firebase reads parameterized config from .env.<project> in the functions source directory.
for (const file of readdirSync(root)) {
  if (file.startsWith('.env.')) {
    copyFileSync(join(root, file), join(outDir, file))
  }
}

// Resolve the external dependencies with the default npm registry, then rewrite resolved URLs to
// the public registry. Integrity hashes are unchanged by the rewrite.
execFileSync(
  'npm',
  ['install', '--package-lock-only', '--ignore-scripts', '--no-audit', '--no-fund'],
  {
    cwd: outDir,
    stdio: 'inherit',
  },
)
const lockPath = join(outDir, 'package-lock.json')
const lock = JSON.parse(readFileSync(lockPath, 'utf8'))
for (const entry of Object.values(lock.packages)) {
  if (typeof entry.resolved === 'string') {
    const marker = entry.resolved.indexOf('/-/')
    const nameEnd = entry.resolved.lastIndexOf('/', marker - 1)
    // Keep "<name>/-/<file>.tgz", including the scope segment for scoped packages.
    const scoped = entry.resolved.slice(0, nameEnd).split('/').pop().startsWith('@')
    const start = scoped ? entry.resolved.lastIndexOf('/', nameEnd - 1) : nameEnd
    entry.resolved = PUBLIC_REGISTRY + entry.resolved.slice(start + 1)
  }
}
writeFileSync(lockPath, JSON.stringify(lock, null, 2) + '\n')
