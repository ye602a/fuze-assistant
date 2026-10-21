// Run with: node --experimental-transform-types tests/electron/test-stage-statistics.js [--live]
// Public dataset only. No Electron startup, LCU discovery, player files or game access.
import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { StageStatisticsService, fetchStageJson } from '../../src/main/services/stage-statistics.ts'

const directory = await mkdtemp(path.join(os.tmpdir(), 'fuze-stage-integration-'))
const live = process.argv.includes('--live')
const row = { id: 1068, winRate: .5, pickRate: .1, sampleCount: 250 }
let requests = 0
const request = async resource => {
  requests++
  if (live) return fetchStageJson(resource)
  if (resource === 'data/versions.json') return { latest: '16.20', versions: [{ version: '16.20', dataPath: 'data/16.20-20261011-abcdef123456', dataDate: '2026-10-11' }] }
  return { champion: { id: 1 }, augments: { all: [row], stages: { 1: [row], 2: [row], 3: [row], 4: [row] } } }
}
try {
  const service = new StageStatisticsService(directory, () => false, request)
  const result = await service.get(1)
  assert.ok(result.snapshot, 'No valid stage snapshot')
  assert.equal(result.snapshot.championId, 1)
  assert.equal(result.snapshot.region, 'unknown')
  for (const stage of ['1', '2', '3', '4']) assert.ok(result.snapshot.rows[stage].length > 0, `No rows for ${stage}`)
  assert.equal(requests, 2)
  const offline = new StageStatisticsService(directory, () => true, async () => { throw new Error('Unexpected request') })
  const cached = await offline.get(1)
  assert.deepEqual(cached.snapshot, result.snapshot)
  console.log(JSON.stringify({ mode: live ? 'public-live' : 'fixture', patch: result.snapshot.patch, date: result.snapshot.dataDate,
    rows: Object.fromEntries(Object.entries(result.snapshot.rows).map(([stage, rows]) => [stage, rows.length])), requests, offlineReadBack: true }))
} finally { await rm(directory, { recursive: true, force: true }) }
