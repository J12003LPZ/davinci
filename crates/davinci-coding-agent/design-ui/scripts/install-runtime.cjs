'use strict';
// Explicit setup only. Never called by /design or by the workspace server.
const fs=require('node:fs'), path=require('node:path'), crypto=require('node:crypto');
const root=path.resolve(process.argv[2]||'');
if(process.argv.length!==3 || fs.existsSync(root))throw new Error('Pass a new absolute runtime directory');
if(!path.isAbsolute(process.argv[2]))throw new Error('Runtime directory must be absolute');
const workspace=path.resolve(__dirname,'../../../..');
const relative=path.relative(workspace,root);
if(relative==='' || (!relative.startsWith('..'+path.sep)&&relative!=='..'&&!path.isAbsolute(relative)))throw new Error('Install outside workspace');
if(process.versions.node!=='24.19.0')throw new Error('Node 24.19.0 required');
const hash=p=>crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const ui=path.resolve(__dirname,'..');
const packages=['react','react-dom','scheduler','esbuild','typescript','playwright-core',`@esbuild/${process.platform}-${process.arch}`];
fs.mkdirSync(root,{recursive:true,mode:0o700});
try {
  for(const name of ['host'])fs.cpSync(path.join(ui,name),path.join(root,name),{recursive:true,dereference:false});
  fs.cpSync(path.join(ui,'../design-resources/ui'),path.join(root,'ui'),{recursive:true,dereference:false});
  for(const name of packages){
    const source=path.dirname(require.resolve(name+'/package.json',{paths:[ui]}));
    fs.mkdirSync(path.dirname(path.join(root,'node_modules',name)),{recursive:true});
    fs.cpSync(source,path.join(root,'node_modules',name),{recursive:true,dereference:false});
  }
  fs.copyFileSync(path.join(ui,'package-lock.json'),path.join(root,'package-lock.json'));
  const files={};
  function visit(directory){
    for(const entry of fs.readdirSync(directory,{withFileTypes:true}).sort((a,b)=>a.name.localeCompare(b.name))){
      const file=path.join(directory,entry.name);
      if(entry.isSymbolicLink())throw new Error('Symlink in runtime');
      if(entry.isDirectory())visit(file);
      else if(entry.isFile())files[path.relative(root,file).split(path.sep).join('/')]=hash(file);
      else throw new Error('Nonregular runtime file');
    }
  }
  visit(root);
  const playwright = require('playwright-core');
  const executable = fs.realpathSync(playwright.chromium.executablePath());
  let directory = path.dirname(executable);
  while (!/^chromium-[0-9]+$/.test(path.basename(directory))) {
    const parent=path.dirname(directory);
    if(parent===directory)throw new Error('Expected the pinned Playwright Chromium cache');
    directory=parent;
  }
  const browserFiles={};
  function visitBrowser(current) {
    for (const entry of fs.readdirSync(current,{withFileTypes:true})) {
      const file=path.join(current,entry.name);
      if(entry.isSymbolicLink())throw new Error('Browser symlink');
      if(entry.isDirectory())visitBrowser(file);
      else if(entry.isFile())browserFiles[path.relative(directory,file).split(path.sep).join('/')]=hash(file);
      else throw new Error('Nonregular browser file');
    }
  }
  visitBrowser(directory);
  const browser={cache:path.dirname(directory),directory,executable,files:browserFiles};
  const fontDirectories=process.platform==='win32'?[path.join(process.env.WINDIR||'C:\\Windows','Fonts')]:process.platform==='darwin'?['/System/Library/Fonts','/Library/Fonts']:['/usr/share/fonts','/usr/local/share/fonts'];
  const fonts=fontDirectories.filter(directory=>fs.existsSync(directory)).map(directory=>{
    const files={};
    function visitFont(current){
      for(const entry of fs.readdirSync(current,{withFileTypes:true})){
        const file=path.join(current,entry.name);
        if(entry.isSymbolicLink())throw new Error('Font symlink');
        if(entry.isDirectory())visitFont(file);
        else if(entry.isFile())files[path.relative(directory,file).split(path.sep).join('/')]=hash(file);
        else throw new Error('Nonregular font file');
      }
    }
    visitFont(directory);return {directory:fs.realpathSync(directory),files};
  });
  if(!fonts.some(font=>Object.keys(font.files).length))throw new Error('No installed fonts to pin');
  fs.writeFileSync(path.join(root,'runtime-manifest.json'),JSON.stringify({schema_version:1,node_version:process.versions.node,node_sha256:hash(process.execPath),files,browser,fonts},null,2)+'\n');
  process.stdout.write(JSON.stringify({directory:root,node:process.execPath,files:Object.keys(files).length})+'\n');
} catch(error) {
  // Preserve a failed installation for inspection; it has no valid manifest.
  process.stderr.write('Runtime setup failed; remove the incomplete directory after inspection.\n');
  throw error;
}
