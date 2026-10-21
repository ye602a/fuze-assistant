import { describe, expect, it } from 'vitest'
import { workbenchHealth } from '../../src/shared/workbench-health.ts'
import type { RuntimeStatus } from '../../src/shared/runtime-status.ts'
const base = { sampledAt: 10000, connection: {discovered:true,verifiedAt:10000,phase:'Lobby',error:null}, automatic:true,
  capture:{running:false,lastCaptureAt:9000},ocr:'idle',data:{state:'ready'} } as RuntimeStatus
describe('player module health', () => {
  it('calls idle lobby modules standby instead of showing old capture as active or an idle engine as broken', () => {
    const result = workbenchHealth(base)
    expect(result.tone).toBe('blue')
    expect(result.modules.slice(1,3).map(m=>m.label)).toEqual(['healthStandby','healthStandby'])
  })
  it('distinguishes healthy saving, GPU fallback and actual capture failure', () => {
    const playing = {...base,connection:{...base.connection,phase:'InProgress'},ocr:'ready',capture:{...base.capture,running:true}} as RuntimeStatus
    expect(workbenchHealth({...playing,levelGate:{mode:'saving'} as any}).tone).toBe('green')
    expect(workbenchHealth({...playing,ocrExecution:{backend:'cpu',fallback:true} as any}).tone).toBe('yellow')
    expect(workbenchHealth({...playing,capture:{...playing.capture,error:'failure'}}).tone).toBe('red')
    expect(workbenchHealth({...base,automatic:false}).tone).toBe('yellow')
  })
  it('does not mark stale connection or failed data as healthy', () => {
    expect(workbenchHealth({...base,sampledAt:40000}).modules[0].tone).toBe('red')
    expect(workbenchHealth({...base,data:{...base.data,state:'error'}}).tone).toBe('red')
    expect(workbenchHealth(base,true).modules.every(m=>m.tone==='red')).toBe(true)
  })
})
