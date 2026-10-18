import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { runInNewContext } from 'node:vm'
import yaml from 'js-yaml'
import { parseSteps, renderSummary } from '../../scripts/actions-validation-summary.mjs'

test('summary reports actual failure even when a step conclusion was overridden', () => {
  const summary = renderSummary('rust', 'failure', { build: { outcome: 'failure', conclusion: 'success' } })
  assert.match(summary, /格式、契约测试与编译 \| 失败/)
  assert.match(summary, /离线 OCR 与原生窗口 \| 未运行或未知/)
})

test('milestone frontend selection builds Slint before bundling without enabling ordinary PR packages', () => {
  const workflow = yaml.load(readFileSync(new URL('../../.github/workflows/rust-prototypes.yml', import.meta.url), 'utf8'))
  assert.equal(workflow.on.workflow_dispatch.inputs.ui_frontend.default, 'slint')
  const steps = workflow.jobs.prototypes.steps
  const frontend = steps.find(step => step.id === 'frontend')
  assert.ok(steps.indexOf(frontend) < steps.findIndex(step => step.id === 'package'))
  assert.match(frontend.run, /--manifest-path rust\/ui-lab\/Cargo.toml/)
  assert.match(frontend.run, /--target-dir rust\/target --release --features gpu --locked/)
  assert.equal(steps.find(step => step.id === 'package').env.FUZE_UI_FRONTEND, '${{ inputs.ui_frontend }}')
  for (const event of ['pull_request', 'push', 'workflow_dispatch']) {
    for (const stage of [true, false]) {
      for (const ui of ['slint', 'win32']) {
        assert.equal(runInNewContext(frontend.if, {
          github: { event_name: event }, inputs: { stage_acceptance: stage, ui_frontend: ui },
        }), event === 'workflow_dispatch' && stage && ui === 'slint')
      }
    }
  }
})

test('summary never includes arbitrary output, account data or error text', () => {
  const privateText = 'private-fixture-do-not-print'
  const summary = renderSummary('electron', privateText, {
    secrets: { outcome: 'success', outputs: { token: privateText } },
    desktop: { outcome: privateText },
    [privateText]: { outcome: 'success' },
    upload_preview: { outcome: 'skipped' },
  })
  assert.ok(!summary.includes(privateText))
  assert.match(summary, /按需测试包 \| 已跳过/)
  assert.match(summary, /安装包窗口验证 \| 未运行或未知/)
  assert.deepEqual(parseSteps('invalid'), {})
  assert.deepEqual(parseSteps('x'.repeat(262145)), {})
  assert.doesNotThrow(() => renderSummary('rust', 'cancelled', null))
})

test('ordinary PRs never package; milestone acceptance preserves package validation', () => {
  for (const name of ['rust-prototypes', 'release-windows']) {
    const workflow = yaml.load(readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8'))
    const steps = Object.values(workflow.jobs)[0].steps
    const uploads = steps.filter((step) => step.uses?.startsWith('actions/upload-artifact@'))
    assert.ok(uploads.length >= 2)
    for (const step of uploads) {
      assert.match(step.if, /github.event_name == 'workflow_dispatch'/)
      assert.match(step.if, /inputs\./)
      assert.match(step.if, /inputs.stage_acceptance/)
      assert.ok(step.with['retention-days'] <= 7)
    }
    for (const id of ['secrets', 'build', 'contracts']) {
      assert.ok(steps.some((step) => step.id === id && !step.if), `${name}: ${id} must still run`)
    }
    assert.equal(workflow.on.workflow_dispatch.inputs.stage_acceptance.default, false)
    const stageSteps = name === 'rust-prototypes' ? ['package', 'desktop'] : ['download', 'package', 'upload_scan', 'desktop']
    for (const id of stageSteps) {
      const expression = steps.find((step) => step.id === id).if
      for (const [event, ref, acceptance, expected] of [
        ['pull_request', 'refs/pull/48/merge', false, false],
        ['pull_request', 'refs/pull/48/merge', true, false],
        ['workflow_dispatch', 'refs/heads/master', false, false],
        ['workflow_dispatch', 'refs/heads/master', true, true],
        ['push', 'refs/heads/master', false, false],
        ['push', 'refs/tags/v0.6.4', false, name === 'release-windows'],
      ]) {
        assert.equal(runInNewContext(expression, {
          github: { event_name: event, ref }, inputs: { stage_acceptance: acceptance },
          startsWith: (value, prefix) => value.startsWith(prefix),
        }), expected, `${name}: ${id}: ${event}/${ref}/${acceptance}`)
      }
    }
    assert.equal(steps.at(-1).if, 'always()')
    if (name === 'rust-prototypes') {
      assert.equal(workflow.on.workflow_dispatch.inputs.artifacts.default, 'none')
      assert.ok(steps.some((step) => step.id === 'build' && step.run.includes('cargo +1.90.0 test --workspace')))
      assert.ok(!steps.find((step) => step.id === 'upload_preview').with.path.includes('*.fuz'))
    } else {
      assert.equal(steps.find((step) => step.id === 'publish').if, "startsWith(github.ref, 'refs/tags/v')")
      assert.ok(steps.findIndex((step) => step.id === 'desktop') < steps.findIndex((step) => step.id === 'publish'))
    }
  }
})

test('Rust integration covers new PR bases without silently skipping jobs or packaging', () => {
  for (const name of ['rust-prototypes', 'ui-framework-lab']) {
    const workflow = yaml.load(readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8'))
    assert.ok(workflow.on.pull_request)
    assert.equal(workflow.on.pull_request.branches, undefined)
    assert.equal(workflow.on.pull_request['branches-ignore'], undefined)
    assert.ok(workflow.on.pull_request.paths.includes('rust/**'))
    assert.ok(workflow.on.pull_request.paths.includes('package-lock.json'))
    for (const job of Object.values(workflow.jobs)) assert.equal(job.if, undefined)
  }
  const ui = yaml.load(readFileSync(new URL('../../.github/workflows/ui-framework-lab.yml', import.meta.url), 'utf8'))
  const steps = ui.jobs['windows-prototype'].steps
  assert.ok(steps.some(step => step.run?.includes('check-rust-screen.mjs') && !step.if))
  assert.ok(steps.some(step => step.run?.includes('--review-host') && !step.if))
  assert.ok(steps.some(step => step.run?.includes('--review-controls') && !step.if))
  assert.ok(steps.some(step => step.run?.includes('--features gpu -j 2') && !step.if))
  assert.ok(!steps.some(step => /package-rust-|gh release/.test(step.run || '')))
  const uploads = steps.filter(step => step.uses?.startsWith('actions/upload-artifact'))
  assert.equal(uploads.length, 1)
  assert.equal(uploads[0].with.name, 'synthetic-ui-screenshots')
  assert.equal(uploads[0].with['retention-days'], 7)
  assert.deepEqual(uploads[0].with.path.trim().split('\n'), [
    'rust/ui-lab/artifacts/restored-*.png',
    'rust/ui-lab/artifacts/direct/direct-*.png',
    'rust/ui-lab/artifacts/host-review/host-*.png',
    'rust/ui-lab/artifacts/control-review/*.png',
    'rust/ui-lab/artifacts/motion/*.png',
  ])
  for (const name of ['rust-lcu-regression', 'rust-detail-startup']) {
    const focused = yaml.load(readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8'))
    assert.equal(focused.on.pull_request, undefined)
    assert.ok(Object.hasOwn(focused.on, 'workflow_dispatch'))
  }
})
