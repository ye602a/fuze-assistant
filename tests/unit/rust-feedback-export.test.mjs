import assert from 'node:assert/strict'
import {mkdtempSync, writeFileSync, readFileSync, rmSync} from 'node:fs'
import {tmpdir} from 'node:os'
import path from 'node:path'
import {spawnSync} from 'node:child_process'
import test from 'node:test'
import {fileURLToPath} from 'node:url'

test('Windows feedback export is bounded, allowlisted and never copies identities or errors', {skip:process.platform !== 'win32'}, () => {
  const dir=mkdtempSync(path.join(tmpdir(),'fuze-feedback-fixture-'))
  const source=path.join(dir,'source.json'), destination=path.join(dir,'feedback.json')
  const script=fileURLToPath(new URL('../../scripts/export-rust-feedback.ps1',import.meta.url))
  const run=(out=destination)=>spawnSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',script,'-Source',source,'-Destination',out],{encoding:'utf8',windowsHide:true,timeout:15000})
  const secret='private-fixture-never-export'
  try {
    const segment={binding:{key:secret,platform:secret,game:987654321},champion:1234,recognition:{analyses:4,partial:1,total_ms:1200,ocr_ms:800,token:secret},interrupted:false,capped:false,events:[{observed_at:123456789,cards:[1111,null,2222],level:6,partial:true,kind:'first',raw:secret},{cards:[null,null,null],level:secret,kind:secret}]}
    const input=JSON.stringify({token:secret,segments:Array.from({length:7},()=>segment)})
    writeFileSync(source,input)
    let result=run()
    assert.equal(result.status,0,result.stdout)
    const raw=readFileSync(destination,'utf8'), report=JSON.parse(raw)
    assert.equal(report.segments.length,5)
    assert.equal(report.segments[0].meanCompletedAnalysisMs,300)
    assert.equal(report.segments[0].meanCompletedOcrMs,200)
    assert.deepEqual(report.segments[0].events[0],{kind:'first',level:6,readableSlots:[true,false,true]})
    assert.equal(report.segments[0].events[1].kind,'unknown')
    assert.equal(report.segments[0].events[1].level,null)
    for(const value of [secret,'987654321','123456789','1111','2222','1234'])assert.ok(!raw.includes(value))
    assert.equal(readFileSync(source,'utf8'),input)
    assert.equal(run().status,1,'never overwrite existing report')
    assert.equal(readFileSync(destination,'utf8'),raw)
    writeFileSync(source,secret)
    result=run(path.join(dir,'invalid.json'))
    assert.equal(result.status,1)
    assert.ok(!(result.stdout+result.stderr).includes(secret))
    writeFileSync(source,JSON.stringify({segments:[]}))
    assert.equal(run(path.join(dir,'empty.json')).status,0)
    assert.deepEqual(JSON.parse(readFileSync(path.join(dir,'empty.json'))).segments,[])
    rmSync(source)
    assert.equal(run(path.join(dir,'missing.json')).status,0)
    assert.equal(JSON.parse(readFileSync(path.join(dir,'missing.json'))).observationStatus,'missing-observations')
  }finally{
    assert.equal(path.dirname(path.resolve(dir)),path.resolve(tmpdir()))
    assert.ok(path.basename(dir).startsWith('fuze-feedback-fixture-'))
    rmSync(dir,{recursive:true,force:true})
  }
})
