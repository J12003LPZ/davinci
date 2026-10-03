// Actual Chromium and trusted control host. The Rust dispatcher is a fixture;
// native confined generated-page execution has a separate Rust integration test.
const {test,expect}=require('@playwright/test');
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
const {createDesignHost}=require('../host/server.cjs');
function assets(){
  const root=path.resolve(__dirname,'../../design-resources/ui');
  return new Map([['/','index.html','text/html'],['/app.js','app.js','application/javascript'],['/app.css','app.css','text/css']].map(([url,file,type])=>[url,{type,body:fs.readFileSync(path.join(root,file))}]));
}
function fixture(){
  const id=crypto.randomUUID(),board=crypto.randomUUID(),node=crypto.randomUUID();
  const manifest={id,title:'Fixture cafe',revision:'1'};
  const revision={revision:'1',variants:[{id:crypto.randomUUID(),title:'Editorial',artboards:[{id:board,title:'Cafe home',entry_point:'index.html'}]}],bindings:[{node_id:node,artboard_id:board,pointer:'/heading',constraint:{type:'text',max_bytes:100},affected_nodes:[node]}]};
  const comments=[],calls=[];let heading='Morning coffee',generationAborted=false;
  const state=()=>({manifest,revision,comments,captures:[],quality:null,runtime_error:'Fixture has no native render evidence',evidence_hash:null,acceptance_gaps:null,accepted:false});
  async function dispatch({operation,payload},signal){
    calls.push(operation);
    switch(operation){
      case 'list':return [manifest];
      case 'status':return state();
      case 'read_binding':return {value:heading,binding_hash:'a'.repeat(64),affected_nodes:[node]};
      case 'edit':
        expect(payload.edit.expected_revision).toBe(manifest.revision);
        heading=payload.edit.value;manifest.revision=String(BigInt(manifest.revision)+1n);revision.revision=manifest.revision;return revision;
      case 'comment':comments.push([payload.comment,false]);return {};
      case 'generate':return new Promise(resolve=>{signal.addEventListener('abort',()=>{generationAborted=true;resolve({});},{once:true});});
      case 'export':return {complete:true,paths:['fixture-only/export'],warning:null};
      case 'close':return {};
      default:throw new Error('Unexpected fixture operation: '+operation);
    }
  }
  return {dispatch,calls,get heading(){return heading;},get aborted(){return generationAborted;}};
}
test('paired workspace edits exact revisions, comments, exports and cancels jobs',async({page})=>{
  const data=fixture();const host=await createDesignHost({assets:assets(),dispatch:data.dispatch});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  try{
    await page.goto(host.url);
    await expect(page.getByText('Connected to this session',{exact:true})).toBeVisible();
    expect(new URL(page.url()).hash).toBe('');
    await page.getByRole('button',{name:'Fixture cafe Revision 1'}).click();
    await expect(page.getByRole('button',{name:'Accept revision',exact:true})).toBeDisabled();
    await page.getByLabel('Editable node',{exact:true}).selectOption({label:'/heading'});
    await expect(page.getByLabel('Value',{exact:true})).toHaveValue('Morning coffee');
    await page.getByLabel('Value',{exact:true}).fill('Coffee for everyone');
    await page.getByRole('button',{name:'Save new revision',exact:true}).click();
    await expect(page.getByLabel('Revision',{exact:true})).toHaveValue('2');
    expect(data.heading).toBe('Coffee for everyone');
    expect(data.calls).not.toContain('generate');
    await page.getByLabel('Feedback on this revision').fill('Keep the accessible heading.');
    await page.getByRole('button',{name:'Add comment',exact:true}).click();
    await expect(page.getByLabel('Saved comments')).toContainText('Keep the accessible heading.');
    await page.getByLabel('New absolute destination directory').fill('/fixture/export');
    await page.getByRole('button',{name:'Export revision',exact:true}).click();
    await expect(page.getByText('Export complete: fixture-only/export')).toBeVisible();
    await page.getByLabel('Revision request',{exact:true}).fill('A quieter layout');
    await page.getByRole('button',{name:'Generate revision',exact:true}).click();
    await expect(page.getByRole('button',{name:'Cancel',exact:true})).toBeVisible();
    await page.getByRole('button',{name:'Cancel',exact:true}).click();
    await expect.poll(()=>data.aborted).toBe(true);
    await expect(page.getByText('Operation cancelled. The last committed revision is preserved.')).toBeVisible();
    expect(errors).toEqual([]);
    await page.screenshot({path:test.info().outputPath('workspace.png'),fullPage:true});
  }finally{await host.close();}
});
test('refresh cannot reuse the consumed pairing capability',async({page})=>{
  const host=await createDesignHost({assets:assets(),dispatch:fixture().dispatch});
  try{
    await page.goto(host.url);await expect(page.getByText('Connected to this session',{exact:true})).toBeVisible();
    await page.reload();await expect(page.getByRole('alert')).toContainText('Pairing link is missing or expired');
    await expect(page.getByRole('button',{name:'Create design',exact:true})).toBeDisabled();
  }finally{await host.close();}
});
