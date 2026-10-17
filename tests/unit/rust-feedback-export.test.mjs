import assert from 'node:assert/strict'
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync} from 'node:fs'
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
    const logs=path.join(dir,'logs');mkdirSync(logs)
    const row={schema:1,event:'state',elapsed_ms:100,dropped:0,state:{connection:'discovery',phase:'unknown',capture:'waiting',ocr:'waiting',issue:'none',failure:'none',stream:'waiting',gate:'fallback',analyses:0,skipped:4,analysis_ms:null,level:null,interval_ms:1000,readable_slots:null,overlay_prepared:false}}
    writeFileSync(path.join(logs,'runtime-v1.1.jsonl'),JSON.stringify({...row,event:'start'})+'\n')
    writeFileSync(path.join(logs,'runtime-v1.jsonl'),JSON.stringify({...row,token:secret})+'\n'+JSON.stringify({...row,state:{...row.state,connection:secret}})+'\n{"partial":')
    let result=run()
    assert.equal(result.status,0,result.stdout)
    const raw=readFileSync(destination,'utf8'), report=JSON.parse(raw)
    assert.equal(report.segments.length,5)
    assert.equal(report.persistentLog.status,'partial')
    assert.equal(report.persistentLog.records.length,2)
    assert.equal(report.persistentLog.skippedLines,2)
    assert.equal(report.persistentLog.records[1].state.connection,'discovery')
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
    assert.equal(result.status,0)
    assert.equal(JSON.parse(readFileSync(path.join(dir,'invalid.json'))).observationStatus,'invalid-observations')
    assert.equal(JSON.parse(readFileSync(path.join(dir,'invalid.json'))).persistentLog.records.length,2)
    assert.ok(!(result.stdout+result.stderr).includes(secret))
    writeFileSync(source,JSON.stringify({segments:[]}))
    assert.equal(run(path.join(dir,'empty.json')).status,0)
    assert.deepEqual(JSON.parse(readFileSync(path.join(dir,'empty.json'))).segments,[])
    rmSync(source)
    assert.equal(run(path.join(dir,'missing.json')).status,0)
    assert.equal(JSON.parse(readFileSync(path.join(dir,'missing.json'))).observationStatus,'missing-observations')
    writeFileSync(path.join(logs,'runtime-v1.2.jsonl'),'x'.repeat(262145))
    assert.equal(run(path.join(dir,'bounded.json')).status,0)
    assert.equal(JSON.parse(readFileSync(path.join(dir,'bounded.json'))).persistentLog.unavailableFiles,1)
    const failures=['window-root','window-layout','window-path-escape','window-access','window-image-unavailable','window-path-mismatch','window-filtered','window-ambiguous','window-discovery']
    writeFileSync(path.join(logs,'runtime-v1.jsonl'),failures.map(failure=>JSON.stringify({...row,state:{...row.state,capture:'failed',issue:'window',failure}})).join('\n')+'\n')
    assert.equal(run(path.join(dir,'window-codes.json')).status,0)
    const windowReport=JSON.parse(readFileSync(path.join(dir,'window-codes.json'))).persistentLog
    assert.equal(windowReport.skippedLines,0)
    assert.deepEqual(windowReport.records.slice(1).map(record=>record.state.failure),failures)
    writeFileSync(path.join(logs,'runtime-v1.jsonl'),JSON.stringify({...row,state:{...row.state,failure:'capture-bitmap',capture_source:'screen-bitmap',capture_fallback:'timeout'}})+'\n'+JSON.stringify({...row,state:{...row.state,capture_source:secret,capture_fallback:secret}})+'\n')
    assert.equal(run(path.join(dir,'screen-source.json')).status,0)
    const screenReport=JSON.parse(readFileSync(path.join(dir,'screen-source.json'))).persistentLog
    assert.equal(screenReport.skippedLines,0)
    assert.equal(screenReport.records[1].state.capture_source,'screen-bitmap')
    assert.equal(screenReport.records[1].state.capture_fallback,'timeout')
    assert.equal(screenReport.records[2].state.capture_source,null)
    assert.equal(screenReport.records[2].state.capture_fallback,null)
    writeFileSync(path.join(logs,'runtime-v1.jsonl'),JSON.stringify({...row,state:{...row.state,title_reads:['exact','no-catalog','low-confidence'],process_cpu_percent:12.5,process_memory_mb:180}})+'\n'+JSON.stringify({...row,state:{...row.state,title_reads:[secret,secret,secret],process_cpu_percent:secret,process_memory_mb:secret}})+'\n')
    assert.equal(run(path.join(dir,'title-reads.json')).status,0)
    const typed=JSON.parse(readFileSync(path.join(dir,'title-reads.json'))).persistentLog.records
    assert.deepEqual(typed[1].state.title_reads,['exact','no-catalog','low-confidence'])
    assert.equal(typed[1].state.process_cpu_percent,12.5)
    assert.deepEqual(typed[2].state.title_reads,['unknown','unknown','unknown'])
    assert.equal(typed[2].state.process_cpu_percent,null)
    assert.ok(!JSON.stringify(typed).includes(secret))
  }finally{
    assert.equal(path.dirname(path.resolve(dir)),path.resolve(tmpdir()))
    assert.ok(path.basename(dir).startsWith('fuze-feedback-fixture-'))
    rmSync(dir,{recursive:true,force:true})
  }
})
