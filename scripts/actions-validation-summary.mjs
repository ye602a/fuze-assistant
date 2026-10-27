import { appendFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

const checks = {
  rust: [
    ['secrets', '源码隐私检查'], ['build', '格式、契约测试与编译'],
    ['ocr', '离线 OCR 与原生窗口'], ['worker', '工作进程与 IPC 生命周期'],
    ['capture', '原生捕获与 GPU ROI'], ['pipeline', '捕获到 OCR 完整链路'],
    ['fallback', 'CPU 对照与回退'], ['contracts', '工作流与数据契约'], ['frontend', '阶段验收：构建与依赖记录'], ['package', '阶段验收：打包、更新与安装回滚'],
    ['desktop', '原生桌面交互'], ['upload_scan', '上传内容隐私检查'],
  ],

}
const delivery = [
  ['upload_preview', '按需测试包'], ['upload_update', '按需更新包'],
  ['upload_evidence', '按需诊断附件'], ['upload_tools', '按需诊断工具'],
  ['publish', '正式 Release 发布'],
]
const labels = { success: '通过', failure: '失败', cancelled: '已取消', skipped: '已跳过' }
const state = (value) => Object.hasOwn(labels, value) ? labels[value] : '未运行或未知'

export function renderSummary(kind, status, steps = {}) {
  if (!Object.hasOwn(checks, kind)) throw new Error('Unsupported workflow kind')
  if (!steps || typeof steps !== 'object' || Array.isArray(steps)) steps = {}
  const rows = (items) => items.map(([id, label]) => `| ${label} | ${state(steps[id]?.outcome)} |`).join('\n')
  return `## FUZE Rust 验证结果\n\n` +
    `作业状态（生成摘要前）：${state(status)}。\n\n` +
    `| 验证项目 | 结果 |\n| --- | --- |\n${rows(checks[kind])}\n\n` +
    `| 可选交付 | 结果 |\n| --- | --- |\n${rows(delivery.filter(([id]) => Object.hasOwn(steps, id)))}\n\n` +
    '普通 PR 只编译和执行代码及离线链路检查，不生成安装包或更新包。完整打包、安装回滚与安装包界面验证仅在明确的阶段验收执行；跳过不代表验收通过。正式版本通过标签发布到 Releases。\n\n' +
    '此摘要只包含固定项目名称和步骤状态，不包含步骤输出或运行数据。离线与自有测试窗口通过不代表真实对局验证，也不代表附件配额已恢复。\n'
}

export function parseSteps(raw = '{}') {
  if (raw.length > 262144) return {}
  try { return JSON.parse(raw) } catch { return {} }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const summary = renderSummary(process.env.FUZE_CI_KIND, process.env.FUZE_CI_STATUS, parseSteps(process.env.FUZE_CI_STEPS))
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary, 'utf8')
  console.log(summary)
}
