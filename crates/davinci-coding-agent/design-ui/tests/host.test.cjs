'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { createDesignHost } = require('../host/server.cjs');
async function request(origin, path, options = {}) {
  const result = await fetch(origin + path, options);
  return { status: result.status, body: await result.text(), headers: result.headers };
}
test('pairing is single-use, origin-bound, and authorization never lives in URLs', async t => {
  let calls = 0;
  const host = await createDesignHost({ dispatch: async () => { calls++; return { artifacts: [] }; }, assets: new Map([['/', { type:'text/html', body:Buffer.from('<h1>Design</h1>') }]]) });
  t.after(() => host.close());
  assert.match(host.url, /#pair=[a-f0-9]{64}$/);
  assert.equal((await request(host.origin, '/api', {method:'POST'})).status, 403);
  const pair = { method:'POST', headers:{ origin:host.origin, 'content-type':'application/json' }, body:JSON.stringify({token:host.pairToken}) };
  assert.equal((await request(host.origin, '/pair', {...pair, headers:{...pair.headers, origin:'https://foreign.invalid'}})).status,403);
  const accepted = await request(host.origin, '/pair', pair);
  assert.equal(accepted.status,200);
  const capability = JSON.parse(accepted.body).capability;
  assert.equal((await request(host.origin, '/pair', pair)).status,403);
  assert.equal((await request(host.origin, '/api?token='+capability, pair)).status,400);
  const call = {method:'POST', headers:{origin:host.origin,'content-type':'application/json',authorization:'Bearer '+capability},body:JSON.stringify({version:1,id:'test-1',operation:'list',payload:{}})};
  assert.equal((await request(host.origin, '/api', call)).status,200);
  assert.equal(calls,1);
  assert.equal((await request(host.origin, '/api', {...call,body:JSON.stringify({version:1,id:'t',operation:'eval',payload:{code:'process.env'}})})).status,400);
  assert.equal((await request(host.origin, '/api', {...call,headers:{...call.headers,'content-type':'text/plain'}})).status,415);
  assert.equal((await request(host.origin, '/api', {method:'OPTIONS',headers:{origin:host.origin}})).status,403);
  host.revoke();
  assert.equal((await request(host.origin, '/api', call)).status,403);
});
test('host header is exact and source files are never static control assets', async t => {
  const host = await createDesignHost({dispatch:async()=>({}),assets:new Map()});
  t.after(()=>host.close());
  const status = await new Promise((resolve,reject)=>{
    const req=http.get(host.origin,{headers:{host:'localhost:'+host.port}},res=>{res.resume();resolve(res.statusCode);});req.on('error',reject);
  });
  assert.equal(status,403);
  assert.equal((await request(host.origin,'/generated/index.html')).status,404);
  assert.equal((await request(host.origin,'/.env')).status,404);
});
test('long generation returns a bounded job and cancellation reaches the existing dispatch', async t => {
  let cancelled=false, finish;
  const host=await createDesignHost({assets:new Map(),dispatch:async(_value,signal)=>{
    signal.addEventListener('abort',()=>{cancelled=true;finish({cancelled:true});},{once:true});
    return new Promise(resolve=>{finish=resolve;});
  }});
  t.after(()=>host.close());
  const paired=await request(host.origin,'/pair',{method:'POST',headers:{origin:host.origin,'content-type':'application/json'},body:JSON.stringify({token:host.pairToken})});
  const capability=JSON.parse(paired.body).capability;
  const send=(operation,payload={})=>request(host.origin,'/api',{method:'POST',headers:{origin:host.origin,'content-type':'application/json',authorization:'Bearer '+capability},body:JSON.stringify({version:1,id:crypto.randomUUID(),operation,payload})});
  const started=await send('generate');
  assert.equal(started.status,202);
  const job=JSON.parse(started.body).job;
  assert.equal((await send('generate')).status,429);
  assert.equal(JSON.parse((await send('job_poll',{job_id:job})).body).result.status,'running');
  assert.equal((await send('job_cancel',{job_id:job})).status,200);
  assert.equal(cancelled,true);
  const result=JSON.parse((await send('job_poll',{job_id:job})).body).result;
  assert.equal(result.status,'cancelled');
  assert.equal((await send('job_poll',{job_id:'forged'})).status,404);
});
test('expired pairing and leases never dispatch and expiration cancels an active job', async t => {
  let now=1_000_000, calls=0, cancelled=false;
  t.mock.method(Date,'now',()=>now);
  const expired=await createDesignHost({assets:new Map(),dispatch:async()=>{calls++;}});
  t.after(()=>expired.close());
  now+=60_001;
  const pair=host=>request(host.origin,'/pair',{method:'POST',headers:{origin:host.origin,'content-type':'application/json'},body:JSON.stringify({token:host.pairToken})});
  assert.equal((await pair(expired)).status,403);
  assert.equal(calls,0);
  const host=await createDesignHost({assets:new Map(),dispatch:async(_value,signal)=>new Promise(resolve=>{
    calls++;
    signal.addEventListener('abort',()=>{cancelled=true;resolve({cancelled:true});},{once:true});
  })});
  t.after(()=>host.close());
  const capability=JSON.parse((await pair(host)).body).capability;
  const send=operation=>request(host.origin,'/api',{method:'POST',headers:{origin:host.origin,'content-type':'application/json',authorization:'Bearer '+capability},body:JSON.stringify({version:1,id:'lease-test',operation,payload:{}})});
  assert.equal((await send('generate')).status,202);
  now+=8*60*60_000+1;
  assert.equal((await send('list')).status,403);
  assert.equal(calls,1);
  assert.equal(cancelled,true);
});
test('revocation cancels ordinary bridge calls as well as long jobs', async t => {
  let started, cancelled=false;
  const ready=new Promise(resolve=>{started=resolve;});
  const host=await createDesignHost({assets:new Map(),dispatch:async(_value,signal)=>new Promise(resolve=>{
    signal.addEventListener('abort',()=>{cancelled=true;resolve({cancelled:true});},{once:true});
    started();
  })});
  t.after(()=>host.close());
  const paired=await request(host.origin,'/pair',{method:'POST',headers:{origin:host.origin,'content-type':'application/json'},body:JSON.stringify({token:host.pairToken})});
  const pending=request(host.origin,'/api',{method:'POST',headers:{origin:host.origin,'content-type':'application/json',authorization:'Bearer '+JSON.parse(paired.body).capability},body:JSON.stringify({version:1,id:'held-read',operation:'list',payload:{}})}).catch(()=>null);
  await ready;
  host.revoke();
  assert.equal(cancelled,true);
  await host.close();
  await pending;
});
test('oversized requests and replies remain bounded', async t => {
  let calls=0;
  const host=await createDesignHost({assets:new Map(),dispatch:async()=>{calls++;return {value:'x'.repeat(256*1024)};}});
  t.after(()=>host.close());
  const paired=await request(host.origin,'/pair',{method:'POST',headers:{origin:host.origin,'content-type':'application/json'},body:JSON.stringify({token:host.pairToken})});
  const send=payload=>request(host.origin,'/api',{method:'POST',headers:{origin:host.origin,'content-type':'application/json',authorization:'Bearer '+JSON.parse(paired.body).capability},body:JSON.stringify({version:1,id:'size-test',operation:'list',payload})});
  assert.equal((await send({value:'x'.repeat(1024*1024)})).status,413);
  assert.equal(calls,0);
  assert.equal((await send({})).status,413);
  assert.equal(calls,1);
});
