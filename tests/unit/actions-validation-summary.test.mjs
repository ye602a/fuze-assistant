import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
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

test('workflows retain checks, gate artifacts to manual requests and preserve tag releases', () => {
  for (const name of ['rust-prototypes', 'release-windows']) {
    const workflow = yaml.load(readFileSync(new URL(`../../.github/workflows/${name}.yml`, import.meta.url), 'utf8'))
    const steps = Object.values(workflow.jobs)[0].steps
    const uploads = steps.filter((step) => step.uses?.startsWith('actions/upload-artifact@'))
    assert.ok(uploads.length >= 2)
    for (const step of uploads) {
      assert.match(step.if, /github.event_name == 'workflow_dispatch'/)
      assert.match(step.if, /inputs\./)
      assert.ok(step.with['retention-days'] <= 7)
    }
    for (const id of ['secrets', 'package', 'desktop', 'upload_scan']) {
      assert.ok(steps.some((step) => step.id === id && !step.if), `${name}: ${id} must still run`)
    }
    assert.equal(steps.at(-1).if, 'always()')
    if (name === 'rust-prototypes') {
      assert.ok(steps.some((step) => step.id === 'build' && step.run.includes('cargo test --workspace')))
      assert.ok(!steps.find((step) => step.id === 'upload_preview').with.path.includes('*.fuz'))
    } else {
      assert.equal(steps.find((step) => step.id === 'publish').if, "startsWith(github.ref, 'refs/tags/v')")
    }
  }
})
