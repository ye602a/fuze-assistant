import test from 'node:test'
import assert from 'node:assert/strict'
import {prepareData} from '../../scripts/prepare-rust-data.mjs'
const failed=category=>({status:1,stderr:`fuze-host: ${category}`})
test('public data transport retries are bounded and successful results still require no game access',async()=>{
 let calls=0;const waits=[]
 const report=await prepareData(timeout=>{assert.ok(timeout>0&&timeout<=360000);return ++calls===1?failed('update-network'):{status:0,stdout:JSON.stringify({data_ready:true,game_access:false})}},{pause:async ms=>waits.push(ms)})
 assert.equal(report.attempts,2);assert.deepEqual(waits,[2000])
 calls=0
 await assert.rejects(prepareData(()=>{calls++;return failed('update-network')},{pause:async()=>{}}),/update-network/)
 assert.equal(calls,3)
})
test('corrupt data, unexpected HTTP responses and incomplete validation never retry',async()=>{
 for(const result of [failed('data-update-hash'),failed('update-http'),{status:0,stdout:'{"data_ready":true,"game_access":true}'}]){
  let calls=0
  await assert.rejects(prepareData(()=>{calls++;return result},{pause:async()=>assert.fail('must not retry')}))
  assert.equal(calls,1)
 }
})
