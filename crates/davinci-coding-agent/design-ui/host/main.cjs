'use strict';
const fs=require('node:fs');
const path=require('node:path');
const {FramedProtocol}=require('./protocol.cjs');
const {createDesignHost}=require('./server.cjs');
const crypto=require('node:crypto');
const protocol=new FramedProtocol(process.stdin,process.stdout);
let host=null, closed=false;
const pending=new Map();
function fail(){if(closed)return;closed=true;for(const p of pending.values())p.reject(new Error('host closed'));pending.clear();if(host)void host.close();process.exitCode=1;process.stdin.destroy();}
protocol.on('error',fail);
protocol.on('closed',fail);
process.on('uncaughtException',fail);
process.on('unhandledRejection',fail);
protocol.on('message',message=>{
  const slot=pending.get(message.id);
  if(!slot || Object.keys(message).some(k=>!['version','id','result','error'].includes(k))) return fail();
  pending.delete(message.id);
  if(slot.cancelled)return;
  if(message.error){const error=new Error('controller denied request');error.detail=message.error;slot.reject(error);}
  else slot.resolve(message.result);
});
(async()=>{
  // The path is written by the Rust supervisor into its private launch directory.
  const config=JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
  const assets=new Map();
  for(const [name,entry] of Object.entries(config.assets)){
    if(!['index.html','app.js','app.css'].includes(name))throw new Error('asset not allowed');
    const body=fs.readFileSync(path.join(config.assetRoot,name));
    if(crypto.createHash('sha256').update(body).digest('hex')!==entry.sha256)throw new Error('asset digest mismatch');
    assets.set(name==='index.html'?'/':'/'+name,{body,type:entry.media_type});
  }
  host=await createDesignHost({assets,dispatch:async(message,signal)=>{
    if(closed || pending.size>=8)throw new Error('controller queue limit');
    return await new Promise((resolve,reject)=>{
      if(pending.has(message.id))return reject(new Error('duplicate request'));
      const abort=()=>{const slot=pending.get(message.id);if(slot){slot.cancelled=true;protocol.send({id:message.id,operation:'cancel'});}reject(new Error('cancelled'));};
      signal.addEventListener('abort',abort,{once:true});
      pending.set(message.id,{resolve:value=>{signal.removeEventListener('abort',abort);resolve(value);},reject:error=>{signal.removeEventListener('abort',abort);reject(error);}});
      protocol.send(message);
      if(signal.aborted)abort();
    });
  }});
  // This capability-bearing URL travels only through the private pipe.
  protocol.send({id:'ready',result:{url:host.url}});
})().catch(fail);
