'use strict';
const esbuild=require('esbuild');
const ts=require('typescript');
const fs=require('node:fs');
const path=require('node:path');
const os=require('node:os');
const crypto=require('node:crypto');
const hash=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
const PACKAGES=['react','react-dom','scheduler'];
const roots=PACKAGES.map(name=>fs.realpathSync(path.dirname(require.resolve(name+'/package.json'))));
const allowedImports=new Map(['react','react/jsx-runtime','react/jsx-dev-runtime','react-dom/client'].map(name=>[name,fs.realpathSync(require.resolve(name))]));
function portable(name) {
  return typeof name==='string' && name.length>0 && name.length<=240 && name.split('/').every(part=>
    /^[a-zA-Z0-9._@-]+$/.test(part) && !['.','..'].includes(part) && !part.endsWith('.') &&
    !/^(con|prn|aux|nul|com[1-9]|lpt[1-9]|conin\$|conout\$)(\.|$)/i.test(part));
}
function validate(input) {
  if (!input || Object.keys(input).sort().join()!=='entry,files' || !input.files || typeof input.files!=='object' || Array.isArray(input.files)) throw new Error('compile schema');
  const files=Object.entries(input.files), seen=new Set();let size=0;
  if (!files.length || files.length>64 || !Object.hasOwn(input.files,input.entry)) throw new Error('file count or entry');
  for (const [name,text] of files) {
    if (!portable(name) || name.toLowerCase()==='__davinci_entry.tsx' || typeof text!=='string' || seen.has(name.toLowerCase())) throw new Error('invalid source path');
    seen.add(name.toLowerCase());const length=Buffer.byteLength(text);size+=length;
    if(length>256*1024 || size>2*1024*1024) throw new Error('source limit');
    if (/\.[cm]?[jt]sx?$/.test(name)) checkImports(name,text);
  }
}
function checkImports(name,text) {
  const file=ts.createSourceFile(name,text,ts.ScriptTarget.ES2022,true);
  function visit(node) {
    if (ts.isCallExpression(node) && (node.expression.kind===ts.SyntaxKind.ImportKeyword ||
        (ts.isIdentifier(node.expression) && node.expression.text==='require'))) {
      if(node.arguments.length!==1 || !ts.isStringLiteral(node.arguments[0])) throw new Error('computed module loading denied');
    }
    ts.forEachChild(node,visit);
  }
  visit(file);
}
function trustedFile(filename) {
  const real=fs.realpathSync(filename);
  if(!roots.some(root=>real.startsWith(root+path.sep)) || !real.endsWith('.js')) throw new Error('runtime file denied');
  return real;
}
async function compile(input,signal) {
  validate(input);
  if(signal?.aborted) throw new Error('cancelled');
  const sourceHash=hash(JSON.stringify(Object.fromEntries(Object.entries(input.files).sort())));
  if(input.entry.endsWith('.html')) return {sourceHash,files:{...input.files},compilerVersion:esbuild.version};
  if(!/\.[jt]sx$/.test(input.entry)) throw new Error('entry must be HTML or TSX/JSX');
  const virtual={...input.files, '__davinci_entry.tsx':
    'import React from "react"; import {createRoot} from "react-dom/client"; import App from "./'+input.entry+'"; createRoot(document.getElementById("root")).render(<App/>);'};
  const plugin={name:'davinci-virtual-files',setup(build){
    build.onResolve({filter:/.*/},args=>{
      if(allowedImports.has(args.path)) return {path:allowedImports.get(args.path),namespace:'trusted'};
      if(args.namespace==='trusted') {
        if(args.path==='react') return {path:allowedImports.get('react'),namespace:'trusted'};
        if(args.path==='scheduler') return {path:trustedFile(require.resolve('scheduler')),namespace:'trusted'};
        if(args.path==='react-dom') return {path:trustedFile(require.resolve('react-dom')),namespace:'trusted'};
        if(!args.path.startsWith('./')) throw new Error('runtime import denied');
        return {path:trustedFile(path.resolve(path.dirname(args.importer),args.path)),namespace:'trusted'};
      }
      const entry=args.kind==='entry-point';
      if(!entry && !args.path.startsWith('./') && !args.path.startsWith('../')) throw new Error('package or external import denied');
      const candidate=entry?args.path:path.posix.normalize(path.posix.join(path.posix.dirname(args.importer),args.path));
      if(!portable(candidate)) throw new Error('virtual import escapes bundle');
      const found=[candidate,candidate+'.tsx',candidate+'.ts',candidate+'.jsx',candidate+'.js',candidate+'.json',candidate+'/index.tsx'].find(p=>Object.hasOwn(virtual,p));
      if(!found) throw new Error('virtual source not found');
      return {path:found,namespace:'source'};
    });
    build.onLoad({filter:/.*/,namespace:'source'},args=>{
      const extension=path.posix.extname(args.path).slice(1);
      const loader={tsx:'tsx',ts:'ts',jsx:'jsx',js:'js',json:'json',css:'css'}[extension];
      if(!loader) throw new Error('unsupported source type');
      return {contents:virtual[args.path],loader};
    });
    build.onLoad({filter:/.*/,namespace:'trusted'},args=>({contents:fs.readFileSync(trustedFile(args.path),'utf8'),loader:'js'}));
  }};
  const context=await esbuild.context({
    absWorkingDir:os.tmpdir(),entryPoints:['__davinci_entry.tsx'],bundle:true,write:false,
    outdir:'davinci-design-output',entryNames:'bundle',format:'iife',platform:'browser',
    target:'es2022',jsx:'automatic',define:{'process.env.NODE_ENV':'"production"'},
    tsconfigRaw:{compilerOptions:{jsx:'react-jsx'}},plugins:[plugin],logLevel:'silent',metafile:true
  });
  let timedOut=false;
  const cancel=()=>{timedOut=true;void context.cancel();};
  const timer=setTimeout(cancel,30000);
  signal?.addEventListener('abort',cancel,{once:true});
  try {
    const result=await context.rebuild();
    if(timedOut || signal?.aborted) throw new Error('compile cancelled');
    if(result.warnings.length) throw new Error('unsupported source construct');
    const files={};
    let total=0;
    for(const output of result.outputFiles){
      total+=output.contents.length;
      if(total>8*1024*1024) throw new Error('compiled output limit');
      files[path.basename(output.path)]=output.text;
    }
    files['index.html']='<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Design preview</title>'+
      (files['bundle.css']?'<link rel="stylesheet" href="./bundle.css">':'')+
      '</head><body><div id="root"></div><script src="./bundle.js"></script></body></html>';
    return {sourceHash,files,compilerVersion:esbuild.version};
  } finally {clearTimeout(timer);signal?.removeEventListener('abort',cancel);await context.dispose();}
}
module.exports={compile,validate};
