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
test('an HTML entry that loads JSX the dev-server way gets a bundle instead',async()=>{
  // What a model wrote in a real run: index.html loading ./src/App.jsx.
  const html='<!doctype html><html><body><div id="root"></div><script type="module" src="./src/App.jsx"></script><script src="./plain.js"></script></body></html>';
  const result=await compile({entry:'index.html',files:{
    'index.html':html,
    'src/App.jsx':'import React from "react"; import {createRoot} from "react-dom/client"; import "./app.css"; function App(){return <h1>Moka</h1>;} createRoot(document.getElementById("root")).render(<App/>);',
    'src/app.css':'h1{color:#3b2f2f}',
    'plain.js':'document.title="plain";',
  }});
  const page=result.files['index.html'];
  assert.doesNotMatch(page,/App\.jsx/);
  assert.match(page,/<script src="\.\/__bundle_0_App\.js" defer><\/script>/);
  assert.match(page,/<link rel="stylesheet" href="\.\/__bundle_0_App\.css">/);
  // A plain script that imports nothing is served as written.
  assert.match(page,/<script src="\.\/plain\.js"><\/script>/);
  assert.match(result.files['__bundle_0_App.js'],/Moka/);
  assert.doesNotMatch(result.files['__bundle_0_App.js'],/from "react"/);
  assert.match(result.files['__bundle_0_App.css'],/#3b2f2f/);
  assert.equal(result.files['src/App.jsx'].includes('<h1>Moka</h1>'),true);
});
test('nested HTML entry bundles beside itself, keeps module deferral and rewrites only src',async()=>{
  const result=await compile({entry:'pages/index.html',files:{
    'pages/index.html':'<head><script data-src="./App.jsx" type="module" src="./App.jsx"></script></head><body><div id="root"></div></body>',
    'pages/App.jsx':'import {createRoot} from "react-dom/client"; import "./app.css"; createRoot(document.getElementById("root")).render("Ready");',
    'pages/app.css':'body{color:#123456}',
  }});
  assert.match(result.files['pages/index.html'],/data-src="\.\/App\.jsx"/);
  assert.match(result.files['pages/index.html'],/<script data-src="\.\/App\.jsx" src="\.\/__bundle_0_App\.js" defer>/);
  assert.match(result.files['pages/index.html'],/href="\.\/__bundle_0_App\.css"/);
  assert.ok(result.files['pages/__bundle_0_App.js']);
  assert.ok(result.files['pages/__bundle_0_App.css']);
  assert.equal(result.files['__bundle_0_App.js'],undefined);
});
test('static imports after other statements still bundle',async()=>{
  const result=await compile({entry:'index.html',files:{
    'index.html':'<script type="module" src="./main.js"></script>',
    'main.js':'console.log(1); import React from "react"; console.log(React.version);',
  }});
  assert.match(result.files['index.html'],/src="\.\/__bundle_0_main\.js" defer/);
  assert.doesNotMatch(result.files['__bundle_0_main.js'],/from "react"/);
});
test('scripts an HTML entry loads are confined like any other source',async()=>{
  for(const path of ['node:fs','https://example.com/a.js','../../outside']) {
    await assert.rejects(()=>compile({entry:'index.html',files:{
      'index.html':'<script type="module" src="./main.tsx"></script>',
      'main.tsx':'import value from '+JSON.stringify(path)+'; console.log(value);',
    }}),path);
  }
  // Remote scripts are left alone here; the page's CSP and the browser's
  // AppContainer refuse them at render.
  const remote=await compile({entry:'index.html',files:{'index.html':'<script src="https://cdn.example.com/x.js"></script>'}});
  assert.match(remote.files['index.html'],/cdn\.example\.com/);
});
