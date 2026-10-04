'use strict';
// The supervisor passes a private config; generated code is only compiled.
const fs=require('node:fs');
const path=require('node:path');
const crypto=require('node:crypto');
const {compile}=require('./compiler.cjs');
const {FramedProtocol}=require('./protocol.cjs');
const protocol=new FramedProtocol(process.stdin,process.stdout);
const abort=new AbortController();
protocol.on('closed',()=>abort.abort());
protocol.on('error',()=>abort.abort());
(async()=>{
  const file=path.resolve(process.argv[2]);
  if(path.dirname(file)!==process.cwd() || fs.statSync(file).size>3*1024*1024)throw new Error('compile config');
  const config=JSON.parse(fs.readFileSync(file,'utf8'));
  const result=await compile(config,abort.signal);
  const bytes=Buffer.from(JSON.stringify(result));
  if(bytes.length>12*1024*1024)throw new Error('compiled output limit');
  fs.writeFileSync(path.join(process.cwd(),'compiled.json'),bytes,{flag:'wx',mode:0o600});
  protocol.send({id:'ready',result:{sha256:crypto.createHash('sha256').update(bytes).digest('hex'),size:bytes.length}});
})().catch(()=>{protocol.send({id:'ready',error:'compile failed'});process.exitCode=1;process.stdin.destroy();});
