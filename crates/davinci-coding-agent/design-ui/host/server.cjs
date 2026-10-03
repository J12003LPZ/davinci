'use strict';
const http = require('node:http');
const crypto = require('node:crypto');
const OPERATIONS = new Set(['list','status','read','read_binding','capture','geometry','create','generate','verify','render','edit','comment','fork','restore','accept','export','apply','draft_implementation','prepare_implementation','sync','cancel','close','job_poll','job_cancel']);
OPERATIONS.add('interact');
const LONG_OPERATIONS = new Set(['generate','verify','render','interact','draft_implementation']);
const MAX_BODY = 1024 * 1024;
const MAX_RESPONSE = 256 * 1024;
const CSP = "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; img-src 'self' blob:; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";
function equal(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string') return false;
  const left = Buffer.from(a), right = Buffer.from(b);
  return left.length === right.length && crypto.timingSafeEqual(left, right);
}
function reply(res, status, value) {
  if (res.destroyed || res.writableEnded) return;
  const bytes = Buffer.from(JSON.stringify(value));
  if (bytes.length > MAX_RESPONSE) return reply(res, 413, {error:'response limit'});
  res.writeHead(status, {'content-type':'application/json; charset=utf-8','content-length':bytes.length});
  res.end(bytes);
}
async function body(req) {
  const parts = []; let length = 0;
  for await (const part of req) {
    length += part.length;
    if (length > MAX_BODY) throw new Error('body limit');
    parts.push(part);
  }
  const result = JSON.parse(Buffer.concat(parts).toString('utf8'));
  if (!result || typeof result !== 'object' || Array.isArray(result)) throw new Error('object required');
  return result;
}
async function createDesignHost({dispatch, assets}) {
  // This module receives only trusted static assets; generated source is never mounted.
  if (!(assets instanceof Map) || typeof dispatch !== 'function') throw new Error('host configuration');
  const pairToken = crypto.randomBytes(32).toString('hex');
  const capability = crypto.randomBytes(32).toString('hex');
  const pairDeadline = Date.now() + 60_000;
  const capabilityDeadline = Date.now() + 8 * 60 * 60_000;
  let paired = false, revoked = false, active = 0, origin = '';
  const jobs=new Map();
  const pendingCalls=new Set();
  let runningJob=false;
  const revoke=()=>{
    revoked=true;
    for(const job of jobs.values())job.abort.abort();
    for(const call of pendingCalls)call.abort();
    jobs.clear();
  };
  const server = http.createServer({maxHeaderSize:16*1024,headersTimeout:5000,requestTimeout:30000}, async (req,res) => {
    res.setHeader('cache-control','no-store');
    res.setHeader('content-security-policy',CSP);
    res.setHeader('x-content-type-options','nosniff');
    res.setHeader('referrer-policy','no-referrer');
    res.setHeader('cross-origin-resource-policy','same-origin');
    res.setHeader('cross-origin-opener-policy','same-origin');
    const raw = req.rawHeaders.filter((_,i)=>i%2===0).map(h=>h.toLowerCase());
    if (['host','origin','authorization','content-type'].some(h=>raw.filter(v=>v===h).length>1) ||
        req.headers.host !== origin.slice('http://'.length)) return reply(res,403,{error:'host denied'});
    if (!req.url || req.url.includes('?') || req.url.includes('#') || req.url.includes('%')) return reply(res,400,{error:'invalid path'});
    if (req.method==='OPTIONS') return reply(res,403,{error:'preflight denied'});
    if (req.method==='GET') {
      if (req.headers.origin && req.headers.origin!==origin) return reply(res,403,{error:'origin denied'});
      if (req.headers['sec-fetch-site'] && !['same-origin','none'].includes(req.headers['sec-fetch-site'])) return reply(res,403,{error:'cross-site denied'});
      const asset=assets.get(req.url);
      if (!asset) return reply(res,404,{error:'not found'});
      res.writeHead(200,{'content-type':asset.type,'content-length':asset.body.length});
      res.end(asset.body); return;
    }
    if (req.method!=='POST' || req.headers.origin!==origin || revoked) return reply(res,403,{error:'origin or capability denied'});
    if (req.headers['content-type']!=='application/json') return reply(res,415,{error:'JSON required'});
    if (Number(req.headers['content-length']||0)>MAX_BODY) return reply(res,413,{error:'body limit'});
    if (!['/pair','/api'].includes(req.url)) return reply(res,404,{error:'not found'});
    if(Date.now()>capabilityDeadline)revoke();
    if (req.url==='/api' && (!paired || revoked || !equal(req.headers.authorization,'Bearer '+capability))) return reply(res,403,{error:'capability expired or invalid'});
    if (active>=8) return reply(res,429,{error:'request queue full'});
    let value;
    try { value=await body(req); } catch { return reply(res,400,{error:'invalid request body'}); }
    if(Date.now()>capabilityDeadline)revoke();
    if(revoked)return reply(res,403,{error:'capability expired or invalid'});
    if (req.url==='/pair') {
      if (paired || Date.now()>pairDeadline || Object.keys(value).join()!=='token' || !equal(value.token,pairToken)) return reply(res,403,{error:'pairing expired or invalid'});
      paired=true;
      return reply(res,200,{capability});
    }
    if (Object.keys(value).sort().join()!=='id,operation,payload,version' || value.version!==1 ||
        typeof value.id!=='string' || !/^[a-zA-Z0-9-]{1,64}$/.test(value.id) || !OPERATIONS.has(value.operation) ||
        !value.payload || typeof value.payload!=='object' || Array.isArray(value.payload)) return reply(res,400,{error:'invalid operation'});
    if (value.operation==='job_poll'||value.operation==='job_cancel') {
      if(Object.keys(value.payload).join()!=='job_id'||typeof value.payload.job_id!=='string')return reply(res,400,{error:'invalid job'});
      const job=jobs.get(value.payload.job_id);
      if(!job)return reply(res,404,{error:'job not found'});
      if(value.operation==='job_cancel'){job.abort.abort();job.status='cancelled';}
      return reply(res,200,{version:1,id:value.id,result:{status:job.status,result:job.result,error:job.error}});
    }
    if(LONG_OPERATIONS.has(value.operation)) {
      if(runningJob)return reply(res,429,{error:'another design operation is running'});
      for(const [id,job] of jobs)if(job.status!=='running'&&jobs.size>=32)jobs.delete(id);
      if(jobs.size>=32)return reply(res,429,{error:'job capacity reached'});
      const id=crypto.randomBytes(16).toString('hex');
      const job={status:'running',abort:new AbortController(),result:undefined,error:undefined};
      jobs.set(id,job);runningJob=true;
      const timer=setTimeout(()=>{job.status='cancelled';job.abort.abort();},600_000);
      Promise.resolve().then(()=>dispatch(value,job.abort.signal)).then(result=>{
        if(job.abort.signal.aborted||revoked){job.status='cancelled';return;}
        if(Buffer.byteLength(JSON.stringify(result))>MAX_RESPONSE-1024)throw new Error('result limit');
        job.result=result;job.status='complete';
      }).catch(error=>{
        if(job.abort.signal.aborted||revoked){job.status='cancelled';return;}
        job.status='failed';job.error=typeof error?.detail?.message==='string'?error.detail.message.slice(0,1024):'Host operation failed';
      }).finally(()=>{clearTimeout(timer);runningJob=false;});
      return reply(res,202,{version:1,id:value.id,job:id});
    }
    if(runningJob && value.operation!=='close')return reply(res,409,{error:'design operation is running; cancel or wait before another action'});
    if(value.operation==='close')for(const job of jobs.values())job.abort.abort();
    if (active>=8) return reply(res,429,{error:'request queue full'});
    active++;
    req.socket.setTimeout(30000);
    const cancellation=new AbortController();
    pendingCalls.add(cancellation);
    const disconnected=()=>{if(!res.writableEnded)cancellation.abort();};
    res.once('close',disconnected);
    const timer=setTimeout(()=>{cancellation.abort();reply(res,504,{error:'request timed out'});},30000);
    try {
      const response=await dispatch(value,cancellation.signal);
      if (!cancellation.signal.aborted && !revoked) reply(res,200,{version:1,id:value.id,result:response});
      else reply(res,403,{error:'capability revoked or request cancelled'});
    } catch(error) {
      const detail=error?.detail;
      if(detail && typeof detail.code==='string' && /^[a-z_]{1,40}$/.test(detail.code))
        reply(res,409,{error:detail.code,message:typeof detail.message==='string'?detail.message.slice(0,1024):''});
      else reply(res,503,{error:cancellation.signal.aborted?'cancelled':'host operation unavailable'});
    }
    finally { clearTimeout(timer);pendingCalls.delete(cancellation);res.removeListener('close',disconnected);active--;req.socket.setTimeout(5000); }
  });
  server.maxConnections=16;
  server.keepAliveTimeout=5000;
  server.setTimeout(5000,socket=>socket.destroy());
  server.on('connect',(_req,socket)=>socket.destroy());
  server.on('upgrade',(_req,socket)=>socket.destroy());
  server.on('clientError',(_err,socket)=>socket.destroy());
  await new Promise((resolve,reject)=>{
    server.once('error',reject);server.listen(0,'127.0.0.1',resolve);
  });
  const port=server.address().port;
  origin='http://127.0.0.1:'+port;
  const leaseTimer=setTimeout(revoke,8*60*60_000);
  leaseTimer.unref();
  return {
    origin,port,pairToken,url:origin+'/#pair='+pairToken,
    revoke,
    async close(){clearTimeout(leaseTimer);revoke();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
  };
}
module.exports={createDesignHost};
