'use strict';
const fs=require('node:fs');
const path=require('node:path');
const esbuild=require('esbuild');
const crypto=require('node:crypto');
const output=path.resolve(__dirname,'../../design-resources/ui');
(async()=>{
  fs.mkdirSync(output,{recursive:true});
  await esbuild.build({absWorkingDir:path.resolve(__dirname,'..'),entryPoints:['src/app.tsx'],bundle:true,minify:true,
    outfile:path.join(output,'app.js'),format:'iife',platform:'browser',target:'es2022',jsx:'automatic',logLevel:'warning',
    define:{'process.env.NODE_ENV':'"production"'}});
  fs.writeFileSync(path.join(output,'index.html'),'<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DaVinci Design workspace</title><link rel="stylesheet" href="/app.css"></head><body><div id="root"></div><script src="/app.js" defer></script></body></html>');
  const assets={};
  for(const [name,media_type] of [['index.html','text/html; charset=utf-8'],['app.js','application/javascript'],['app.css','text/css']]){
    assets[name]={sha256:crypto.createHash('sha256').update(fs.readFileSync(path.join(output,name))).digest('hex'),media_type};
  }
  fs.writeFileSync(path.join(output,'manifest.json'),JSON.stringify(assets,null,2)+'\n');
})();
