'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const { compile }=require('../host/compiler.cjs');
test('virtual TSX compiles without executing generated modules',async()=>{
  const result=await compile({entry:'App.tsx',files:{'App.tsx':'import React from "react"; export default function App(){ throw new Error("must never run in Node"); return <h1>Hello</h1>; }'}});
  assert.equal(result.sourceHash.length,64);
  assert.match(result.files['bundle.js'],/must never run in Node/);
  assert.match(result.files['index.html'],/bundle.js/);
});
test('project files, configuration, native modules, and network imports are inaccessible',async()=>{
  for(const path of ['node:fs','fs','https://example.com/a.js','../../outside','/etc/passwd','unknown-package']) {
    await assert.rejects(()=>compile({entry:'App.tsx',files:{'App.tsx':'import value from '+JSON.stringify(path)+'; export default value;'}}),path);
  }
  await assert.rejects(()=>compile({entry:'App.tsx',files:{'App.tsx':'export default import(location.hash);'}}));
  await assert.rejects(()=>compile({entry:'App.tsx',files:{'App.tsx':'export default require(globalThis.name);'}}));
});
test('portable paths and size bounds also hold at compiler boundary',async()=>{
  await assert.rejects(()=>compile({entry:'../x.html',files:{'../x.html':'hello'}}));
  await assert.rejects(()=>compile({entry:'index.html',files:{'index.html':'x'.repeat(256*1024+1)}}));
  await assert.rejects(()=>compile({entry:'index.html',files:{'index.html':'ok','INDEX.HTML':'collision'}}));
});
