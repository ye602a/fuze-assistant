import assert from 'node:assert/strict'
import * as fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { runInNewContext } from 'node:vm'
import yaml from 'js-yaml'

const root = fileURLToPath(new URL('../../', import.meta.url))
const readYaml = file => yaml.load(fs.readFileSync(path.join(root, file), 'utf8'))
const workflow = name => readYaml(`.github/workflows/${name}.yml`)
const cacheAction = './.github/actions/rust-build-cache'
const require = createRequire(import.meta.url)

// Exercise the actual key generator with a fixed compiler, without invoking Rust or writing files.
function identity({ lane = 'core', env = {}, files = {}, compiler = 'rustc 1.90.0\nhost: x86_64-pc-windows-msvc' } = {}) {
  const action = readYaml('.github/actions/rust-build-cache/action.yml')
  const script = action.runs.steps.find(step => step.id === 'identity').run.match(/@'\r?\n([\s\S]*?)\r?\n'@ \| node/)[1]
  let output = ''
  runInNewContext(script, {
    require(name) {
      if (name === 'node:child_process') return { execFileSync(command, args) {
        assert.equal(command, 'rustc')
        assert.deepEqual([...args], ['+1.90.0', '-vV'])
        return compiler
      } }
      if (name === 'node:fs') return { ...fs,
        readFileSync(file, encoding) { return files[path.relative(root, file).replaceAll('\\', '/')] ?? fs.readFileSync(file, encoding) },
        appendFileSync(_file, text) { output += text },
      }
      return require(name)
    },
    process: { env: {
      GITHUB_WORKSPACE: root, GITHUB_OUTPUT: 'unused', FUZE_CACHE_LANE: lane,
      RUNNER_OS: 'Windows', RUNNER_ARCH: 'X64', ImageOS: 'win22', ImageVersion: '20261101.1',
      CARGO_HOME: 'C:/cargo', GITHUB_SHA: 'source-one', ...env,
    } },
    console: { log() {} },
  }, { timeout: 1000 })
  return {
    key: output.match(/^key=(.+)$/m)[1],
    prefix: output.match(/^prefix=(.+)$/m)[1],
    paths: output.match(/^paths<<(\S+)\n([\s\S]*?)\n\1$/m)[2].split('\n'),
  }
}

function enabled(step, context, success = true) {
  const expression = (step.if || 'true').replace(/^\$\{\{\s*|\s*\}\}$/g, '')
    .replace(/\bsteps\.([\w-]+)\.outputs\.([\w-]+)/g, 'steps["$1"].outputs["$2"]')
    .replace(/\bsteps\.([\w-]+)\.outcome/g, 'steps["$1"].outcome')
  // Actions adds success() to conditions that do not contain a status function.
  if (!success && !/\b(?:always|success|failure|cancelled)\(/.test(expression)) return false
  return runInNewContext(expression, { ...context, success: () => success, always: () => true })
}

test('Rust cache keys reuse source changes and isolate dependencies, lanes and compiler environments', () => {
  const action = readYaml('.github/actions/rust-build-cache/action.yml')
  assert.equal(action.inputs['lookup-only'].default, 'false')
  assert.equal(action.outputs['cache-hit'].value, '${{ steps.restore.outputs.cache-hit }}')
  assert.equal(action.outputs['cache-key'].value, '${{ steps.identity.outputs.key }}')
  assert.equal(action.outputs['cache-paths'].value, '${{ steps.identity.outputs.paths }}')
  const restore = action.runs.steps.find(step => step.uses?.startsWith('actions/cache/restore@'))
  assert.ok(restore)
  assert.equal(restore.with['lookup-only'], '${{ inputs.lookup-only }}')
  assert.equal(restore.with['restore-keys'], '${{ steps.identity.outputs.prefix }}')
  assert.ok(action.runs.steps.every(step => !/^actions\/cache(?:@|\/save@)/.test(step.uses || '')))
  const base = identity()
  assert.equal(identity({ env: { GITHUB_SHA: 'source-two' }, files: { 'rust/app-version.txt': '0.7.99' } }).key, base.key)
  const changedDependency = identity({ files: { 'rust/Cargo.lock': 'different dependency graph' } })
  assert.notEqual(changedDependency.key, base.key)
  assert.equal(changedDependency.prefix, base.prefix)
  for (const change of [
    { lane: 'ui' }, { lane: 'publish' }, { compiler: 'different rustc build' },
    { env: { ImageVersion: '20261102.1' } }, { env: { RUSTFLAGS: '-C target-cpu=native' } },
  ]) assert.notEqual(identity(change).prefix, base.prefix)
  for (const lane of ['core', 'ui', 'publish']) {
    const target = lane === 'ui' ? 'rust/ui-lab/target/release' : 'rust/target/release'
    const paths = identity({ lane }).paths
    assert.ok(paths.includes('C:/cargo/registry/cache'))
    for (const directory of ['.fingerprint', 'build', 'deps']) assert.ok(paths.includes(`${target}/${directory}`))
    assert.ok(!paths.includes(target), 'tar must not recursively include final executables or build receipts')
    assert.ok(paths.every(entry => !/artifacts|incremental|build-evidence|\.(exe|pdb)$/.test(entry)))
  }
})

test('PR core and UI checks only restore caches and still compile and test every source revision', () => {
  for (const [name, lane, builds] of [
    ['rust-prototypes', 'core', ['build']],
    ['ui-framework-lab', 'ui', ['native-build', 'production-build']],
  ]) {
    const steps = Object.values(workflow(name).jobs)[0].steps
    assert.ok(steps.some(step => step.uses === cacheAction && step.with.lane === lane && !step.if))
    assert.ok(steps.every(step => !/^actions\/cache(?:@|\/save@)/.test(step.uses || '')))
    assert.ok(steps.every(step => !(step.if || '').includes('cache-hit')))
    for (const id of builds) {
      const build = steps.find(step => step.id === id)
      assert.equal(build.if, undefined)
      assert.match(build.run, /cargo \+1\.90\.0 build/)
      if (id !== 'production-build') {
        assert.match(build.run, /cargo \+1\.90\.0 test/)
        assert.doesNotMatch(build.run, /--no-run/)
      }
    }
  }
})

test('master cache warming skips exact hits, compiles misses without running tests, and never saves failed builds', () => {
  const warm = workflow('rust-cache-warm')
  assert.deepEqual(Object.keys(warm.on).sort(), ['push', 'workflow_dispatch'])
  assert.deepEqual(warm.on.push.branches, ['master'])
  assert.ok(warm.on.push.paths.includes('rust/**'))
  assert.ok(warm.on.push.paths.includes('.github/actions/rust-build-cache/**'))
  const job = Object.values(warm.jobs)[0]
  for (const ref of ['refs/heads/master', 'refs/heads/feature', 'refs/pull/42/merge']) {
    assert.equal(enabled(job, { github: { ref } }), ref === 'refs/heads/master')
  }
  assert.deepEqual(job.strategy.matrix.lane, ['core', 'ui'])
  const lookup = job.steps.find(step => step.id === 'lookup')
  const restore = job.steps.find(step => step.id === 'restore')
  const save = job.steps.find(step => step.uses?.startsWith('actions/cache/save@'))
  assert.equal(lookup.uses, cacheAction)
  assert.equal(lookup.with['lookup-only'], 'true')
  assert.equal(restore.uses, cacheAction)
  assert.notEqual(restore.with['lookup-only'], 'true')
  assert.equal(save.with.key, '${{ steps.restore.outputs.cache-key }}')
  assert.equal(save.with.path, '${{ steps.restore.outputs.cache-paths }}')
  for (const lane of ['core', 'ui']) {
    const build = job.steps.find(step => step.id === `${lane}-build`)
    const otherBuild = job.steps.find(step => step.id === `${lane === 'core' ? 'ui' : 'core'}-build`)
    const testCommands = build.run.split('\n').filter(line => /^cargo .*\btest\b/.test(line))
    assert.ok(testCommands.length)
    for (const command of testCommands) assert.match(command, /--no-run/)
    for (const [lookupHit, restoreHit, outcome, shouldBuild, shouldSave] of [
      ['true', '', 'skipped', false, false],
      ['', 'false', 'success', true, true],
      ['false', 'true', 'skipped', false, false],
      ['false', 'false', 'failure', true, false],
    ]) {
      const context = { matrix: { lane }, steps: {
        lookup: { outputs: { 'cache-hit': lookupHit } }, restore: { outputs: { 'cache-hit': restoreHit } },
        [`${lane}-build`]: { outcome }, [`${lane === 'core' ? 'ui' : 'core'}-build`]: { outcome: 'skipped' },
      } }
      assert.equal(enabled(restore, context), lookupHit !== 'true')
      assert.equal(enabled(build, context), shouldBuild)
      assert.equal(enabled(otherBuild, context), false)
      assert.equal(enabled(save, context, outcome !== 'failure'), shouldSave)
    }
  }
  assert.doesNotMatch(JSON.stringify(job.steps), /package-rust|check-rust|upload-artifact|--review|--desktop-smoke|fuze-[\w-]+\.exe/)
})

test('manual releases compile exact source and save successful dependencies before full package validation', () => {
  const release = workflow('publish-rust')
  assert.deepEqual(Object.keys(release.on), ['workflow_dispatch'])
  const job = release.jobs.preview
  assert.equal(enabled(job, { github: { ref: 'refs/heads/master' } }), true)
  assert.equal(enabled(job, { github: { ref: 'refs/heads/feature' } }), false)
  const buildIndex = job.steps.findIndex(step => step.id === 'native-build')
  const build = job.steps[buildIndex]
  assert.equal(build.if, undefined)
  assert.match(build.run, /git rev-parse HEAD/)
  assert.match(build.run, /build-rust-with-evidence\.ps1/)
  const save = job.steps[buildIndex + 1]
  assert.match(save.uses, /^actions\/cache\/save@/)
  assert.equal(save['continue-on-error'], true)
  for (const hit of ['true', 'false', '']) {
    for (const outcome of ['success', 'failure', 'skipped']) {
      assert.equal(enabled(save, { steps: {
        'native-build': { outcome }, 'rust-cache': { outputs: { 'cache-hit': hit } },
      } }, outcome !== 'failure'), outcome === 'success' && hit !== 'true')
    }
  }
  for (const script of ['prepare-rust-data.mjs', 'package-rust-preview.mjs', 'finalize-preview.mjs', 'package-rust-update.mjs', 'check-rust-update.mjs', 'publish-preview.mjs']) {
    const index = job.steps.findIndex(step => step.run?.includes(script))
    assert.ok(index > buildIndex + 1, `${script} must follow compilation and cache save`)
    assert.equal(job.steps[index].if, undefined)
  }
})
