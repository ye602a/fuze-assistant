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
      assert.ok(steps.some((step) => step.id === 'build' && step.run.includes('cargo test --workspace')))
      assert.ok(!steps.find((step) => step.id === 'upload_preview').with.path.includes('*.fuz'))
    } else {
      assert.equal(steps.find((step) => step.id === 'publish').if, "startsWith(github.ref, 'refs/tags/v')")
      assert.ok(steps.findIndex((step) => step.id === 'desktop') < steps.findIndex((step) => step.id === 'publish'))
    }
  }
})
