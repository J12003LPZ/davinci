import React, {useEffect, useRef, useState} from "react";
import {createRoot} from "react-dom/client";
import {pair, call as hostCall, captureBlob} from "./api";
import {InteractionPanel} from "./InteractionPanel";
import type {ArtifactManifest, Artboard, DesignComment, DesignRevision, DesignRunState, EditableBinding, ExportFormat, ExportReceipt, InteractionReceipt, PrototypeAction, PreparedHandoff, QualityReport, Viewport} from "./contracts.generated";
import "./style.css";

type Capture = {artboard_id:string; viewport:Viewport; sha256:string};
type GeometryNode = {id:string; x:number; y:number; width:number; height:number; visible:boolean};
type ArtifactState = {manifest: ArtifactManifest; revision: DesignRevision | null; quality: QualityReport | null; captures: Capture[]; comments:[DesignComment,boolean][];
  runtime_error:string|null; evidence_hash:string|null; acceptance_gaps:string[]|null; accepted:boolean};
function App() {
  const [ready,setReady]=useState(false), [busy,setBusy]=useState(false), [error,setError]=useState("");
  const [artifacts,setArtifacts]=useState<ArtifactManifest[]>([]), [selected,setSelected]=useState<ArtifactState|null>(null);
  const [board,setBoard]=useState<Artboard|null>(null),[binding,setBinding]=useState<EditableBinding|null>(null);
  const [brief,setBrief]=useState(""),[comment,setComment]=useState(""),[value,setValue]=useState("");
  const [zoom,setZoom]=useState(100),[viewport,setViewport]=useState("390×844"),[mode,setMode]=useState<"edit"|"interact">("edit");
  const [revision,setRevision]=useState(""),[notice,setNotice]=useState("");
  const [destination,setDestination]=useState(""),[exportFormat,setExportFormat]=useState<ExportFormat>("source"),[acknowledged,setAcknowledged]=useState(false);
  const [image,setImage]=useState(""),[geometry,setGeometry]=useState<GeometryNode[]>([]);
  const [targets,setTargets]=useState(""),[proposal,setProposal]=useState<PreparedHandoff|null>(null);
  const [interaction,setInteraction]=useState<InteractionReceipt|null>(null);
  const pending=useRef<AbortController|null>(null);
  function call<T>(operation:string,payload:object):Promise<T>{return hostCall<T>(operation,payload,pending.current?.signal);}
  async function run(action:()=>Promise<void>) {
    if(pending.current)return;
    setBusy(true);setError("");const abort=new AbortController();pending.current=abort;
    try {await action();} catch(e) {if(abort.signal.aborted)setNotice("Operation cancelled. The last committed revision is preserved.");else setError(e instanceof Error?e.message:"Operation failed");}
    finally {pending.current=null;setBusy(false);}
  }
  async function refresh(){setArtifacts(await call<ArtifactManifest[]>("list",{}));}
  async function open(id:string){
    const state=await call<ArtifactState>("status",{artifact_id:id});
    setSelected(state);setBoard(state.revision?.variants[0]?.artboards[0]||null);setBinding(null);setRevision(state.manifest.revision);
    setAcknowledged(false);
    setProposal(null);
  }
  useEffect(()=>{let active=true;void pair().then(async()=>{if(!active)return;setReady(true);await refresh();}).catch(e=>setError(String(e)));return()=>{active=false;pending.current?.abort();};},[]);
  useEffect(()=>setInteraction(null),[selected?.manifest.id,selected?.manifest.revision,board?.id,viewport]);
  const baseline=selected?.captures.find(c=>c.artboard_id===board?.id && c.viewport.width+"×"+c.viewport.height===viewport);
  const prototype=mode==="interact"&&interaction?.request.render.artboard_id===board?.id&&interaction?.request.render.revision===selected?.manifest.revision?interaction:null;
  const capture=prototype?{artboard_id:prototype.request.render.artboard_id,viewport:prototype.request.render.viewport,sha256:prototype.capture.screenshot.sha256}:baseline;
  const captureHash=capture?.sha256;
  useEffect(()=>{
    setImage("");setGeometry([]);
    if(busy||!capture||!selected)return;
    const abort=new AbortController();let imageUrl="";
    const payload={artifact_id:selected.manifest.id,revision:selected.manifest.revision,artboard_id:capture.artboard_id,viewport:capture.viewport,interaction_id:prototype?.request.operation_id??null};
    void Promise.all([captureBlob(payload,capture.sha256,abort.signal),hostCall<{nodes:GeometryNode[]}>("geometry",payload,abort.signal)])
      .then(([blob,info])=>{if(abort.signal.aborted)return;imageUrl=URL.createObjectURL(blob);setImage(imageUrl);setGeometry(info.nodes);})
      .catch(e=>{if(!abort.signal.aborted)setError(String(e));});
    return()=>{abort.abort();if(imageUrl)URL.revokeObjectURL(imageUrl);};
  },[captureHash,selected?.manifest.id,selected?.manifest.revision,prototype?.request.operation_id,busy]);
  useEffect(()=>{
    if(busy||!binding||!selected)return;
    const abort=new AbortController();
    void hostCall<{value:unknown}>("read_binding",{artifact_id:selected.manifest.id,revision:selected.manifest.revision,node_id:binding.node_id},abort.signal)
      .then(result=>setValue(String(result.value))).catch(e=>{if(!abort.signal.aborted)setError(String(e));});
    return()=>abort.abort();
  },[binding,selected?.manifest.id,selected?.manifest.revision,busy]);
  async function verify(){
    if(!selected?.revision)return;
    await call("verify",{artifact_id:selected.manifest.id});
    await open(selected.manifest.id);
  }
  async function interact(action:PrototypeAction){
    if(!selected?.revision||!board)return;
    const [width,height]=viewport.split("×").map(Number);
    const result=await call<InteractionReceipt>("interact",{request:{render:{artifact_id:selected.manifest.id,revision:selected.manifest.revision,artboard_id:board.id,viewport:{width,height},theme:"light",fixture:"default",reduced_motion:true},
      actions:[...(interaction?.request.actions??[]),action],operation_id:crypto.randomUUID()}});
    setInteraction(result);setNotice("Prototype sequence: "+result.capture.checks.interaction.state+". "+result.capture.checks.interaction.failures.join(" "));
  }
  async function generate(item:ArtifactManifest,brief:string){
    const state=await call<DesignRunState>("generate",{request:{artifact_id:item.id,expected_revision:item.revision,brief,operation_id:crypto.randomUUID()}});
    await refresh();await open(item.id);
    setNotice(state.progress.phase==="complete"?"Source generation complete. Review the separate evidence checks before acceptance.":"Generation stopped; revision "+state.last_revision+" is retained. "+state.findings.join(" "));
  }
  const bindings=selected?.revision?.bindings.filter(b=>b.artboard_id===board?.id)||[];
  const dimensions=selected?.quality ? Object.entries(selected.quality).filter(([key])=>!["revision","evidence"].includes(key)): [];
  return <main>
    <header><div><span className="eyebrow">DAVINCI</span><h1>Design workspace</h1></div><p role="status">{busy?"Working…":ready?"Connected to this session":"Connecting…"}</p>
      {busy && <button onClick={()=>pending.current?.abort()}>Cancel</button>}
      <button disabled={busy||!ready} onClick={()=>void run(async()=>{await call("close",{});setReady(false);setNotice("Workspace closed. Reopen it from DaVinci.");})}>Close workspace</button></header>
    {error && <div role="alert" className="error">{error}<button onClick={()=>setError("")}>Dismiss</button></div>}
    {notice && <p role="status" className="notice">{notice}</p>}
    <div className="workspace" aria-busy={busy}>
      <aside aria-label="Artifacts and artboards"><h2>Artifacts</h2>
        <form onSubmit={e=>{e.preventDefault();void run(async()=>{
          const item=await call<ArtifactManifest>("create",{input:{title:brief.slice(0,80),brief,kind:"landing",variants:2,operation_id:crypto.randomUUID()}});
          await refresh();await open(item.id);await generate(item,brief);setBrief("");
        });}}><label htmlFor="brief">New design brief</label><textarea id="brief" required maxLength={65536} value={brief} onChange={e=>setBrief(e.target.value)}/>
          <button disabled={!ready||busy}>Create design</button></form>
        <nav aria-label="Artifact list">{artifacts.map(a=><button key={a.id} aria-current={selected?.manifest.id===a.id?"page":undefined} onClick={()=>void run(()=>open(a.id))}>{a.title}<small>Revision {a.revision}</small></button>)}</nav>
        {selected&&<form onSubmit={e=>{e.preventDefault();void run(()=>generate(selected.manifest,brief));}}><label htmlFor="revision-brief">Revision request</label><textarea id="revision-brief" required maxLength={65536} value={brief} onChange={e=>setBrief(e.target.value)}/><button disabled={!ready||busy}>Generate revision</button></form>}
        {selected?.revision?.variants.map(v=><section key={v.id}><h3>{v.title}</h3><ul>{v.artboards.map(b=><li key={b.id}><button disabled={busy} aria-pressed={board?.id===b.id} onClick={()=>{setBoard(b);setBinding(null);}}>{b.title}</button></li>)}</ul></section>)}
      </aside>
      <section className="canvas" aria-label="Artboard preview">
        <div className="toolbar"><div role="group" aria-label="Workspace mode"><button disabled={busy} aria-pressed={mode==="edit"} onClick={()=>setMode("edit")}>Edit</button><button disabled={busy} aria-pressed={mode==="interact"} onClick={()=>setMode("interact")}>Interact</button></div>
          <label>Viewport<select disabled={busy} value={viewport} onChange={e=>setViewport(e.target.value)}>{["390×844","768×1024","1440×900"].map(v=><option key={v}>{v}</option>)}</select></label>
          <label>Zoom <input aria-label="Zoom" type="range" min={25} max={200} step={25} value={zoom} onChange={e=>setZoom(Number(e.target.value))}/>{zoom}%</label>
        </div>
        <div className="artboard-scroll" tabIndex={0} aria-label="Scrollable artboard">
          {image&&capture?<div className="capture-frame" style={{width:capture.viewport.width*zoom/100,height:capture.viewport.height*zoom/100}}>
              <img className="capture" width={capture.viewport.width*zoom/100} height={capture.viewport.height*zoom/100} src={image} alt={board?.title+" at "+viewport}/>
              {mode==="edit"&&geometry.filter(node=>node.visible&&bindings.some(item=>item.node_id===node.id)).map(node=><button key={node.id}
                className="node-overlay" aria-label={"Select "+(bindings.find(item=>item.node_id===node.id)?.pointer||node.id)}
                aria-pressed={binding?.node_id===node.id} style={{left:node.x*zoom/100,top:node.y*zoom/100,width:node.width*zoom/100,height:node.height*zoom/100}}
                onClick={()=>setBinding(bindings.find(item=>item.node_id===node.id)||null)}/>)}
            </div>:
              <div className="empty"><span className="eyebrow">SOURCE-BACKED DESIGN</span><h2>{selected?board?.title||selected.manifest.title:"A place to develop your next idea"}</h2><p>{selected?"A verified capture will appear here. Source and evidence status remain separate.":"Create a design or choose an existing artifact. Compare concepts, refine declared properties, and keep every revision."}</p></div>}
        </div>
        <footer>{selected?<><strong>{selected.manifest.title}</strong><span>Revision {selected.manifest.revision}</span><button disabled={busy||!selected.revision} onClick={()=>void run(verify)}>Verify</button></>:<span>No artifact selected</span>}</footer>
      </section>
      <aside aria-label="Inspector and comments"><h2>Inspector</h2><label htmlFor="editable-node">Editable node</label><select id="editable-node" value={binding?.node_id||""} onChange={e=>{setBinding(bindings.find(b=>b.node_id===e.target.value)||null);setValue("");}}><option value="">Select from list</option>{bindings.map(b=><option key={b.node_id} value={b.node_id}>{b.pointer}</option>)}</select>
        {mode==="interact"&&<InteractionPanel busy={busy||!selected?.revision||!board} count={interaction?.request.actions.length??0} onAction={action=>void run(()=>interact(action))} onReset={()=>setInteraction(null)}/>}
        {binding && <form onSubmit={e=>{e.preventDefault();void run(async()=>{
          const preview=await call<{binding_hash:string;affected_nodes:string[]}>("read_binding",{artifact_id:selected!.manifest.id,revision:selected!.manifest.revision,node_id:binding.node_id});
          if(preview.affected_nodes.length>1 && !window.confirm("This shared token affects "+preview.affected_nodes.length+" nodes. Apply to all?"))return;
          await call("edit",{edit:{artifact_id:selected!.manifest.id,expected_revision:selected!.manifest.revision,node_id:binding.node_id,expected_binding_hash:preview.binding_hash,operation_id:crypto.randomUUID(),value:binding.constraint.type==="number"?Number(value):value,confirmed_affected_nodes:preview.affected_nodes}});
          await open(selected!.manifest.id);
        });}}><label>Value<input value={value} onChange={e=>setValue(e.target.value)} required/></label><button disabled={busy}>Save new revision</button></form>}
        <h2>Comments</h2><form onSubmit={e=>{e.preventDefault();void run(async()=>{
          await call("comment",{operation_id:crypto.randomUUID(),comment:{id:crypto.randomUUID(),artifact_id:selected!.manifest.id,revision:selected!.manifest.revision,artboard_id:board!.id,node_id:binding?.node_id||null,x_milli:0,y_milli:0,text:comment}});
          setComment("");setNotice("Comment saved on revision "+selected!.manifest.revision);await open(selected!.manifest.id);
        });}}><label htmlFor="comment">Feedback on this revision</label><textarea id="comment" value={comment} maxLength={8192} onChange={e=>setComment(e.target.value)} required/><button disabled={!board||busy}>Add comment</button></form>
        <ul aria-label="Saved comments">{selected?.comments.map(([item,orphaned])=><li key={item.id}><p>{item.text}</p><small>Revision {item.revision}{orphaned?" · Anchor no longer present":""}</small></li>)}</ul>
        <h2>Evidence</h2>{selected?.runtime_error&&<p className="notice">{selected.runtime_error}</p>}
        {dimensions.map(([name,dimension])=><div className="evidence" key={name}><strong>{name}</strong><span>{(dimension as {state:string}).state}</span></div>)}
        {selected && <><h2>History</h2><label>Revision<input inputMode="numeric" pattern="[1-9][0-9]*" value={revision} onChange={e=>setRevision(e.target.value)}/></label><div className="actions">{(["fork","restore"] as const).map(op=><button key={op} disabled={busy||!selected.revision} onClick={()=>void run(async()=>{
          const result=await call<ArtifactManifest|DesignRevision>(op,{artifact_id:selected.manifest.id,revision,operation_id:crypto.randomUUID(),...(op==="restore"?{expected_revision:selected.manifest.revision}:{})});
          await refresh();await open(op==="fork"?(result as ArtifactManifest).id:selected.manifest.id);
        })}>{op==="fork"?"Fork":"Restore"}</button>)}</div>
          <h2>Delivery</h2><p>{selected.accepted?"This revision is accepted. Implementation remains a separate coding task.":"Accept a revision after reviewing its captures and evidence."}</p>
          {!!selected.acceptance_gaps?.length&&<label><input type="checkbox" checked={acknowledged} onChange={e=>setAcknowledged(e.target.checked)}/>
            I reviewed the incomplete checks: {selected.acceptance_gaps.join(", ")}</label>}
          <button disabled={busy||selected.accepted||!selected.acceptance_gaps||(selected.acceptance_gaps.length>0&&!acknowledged)} onClick={()=>void run(async()=>{
            await call("accept",{request:{artifact_id:selected.manifest.id,revision:selected.manifest.revision,evidence_hash:selected.evidence_hash,
              acknowledged_incomplete:selected.acceptance_gaps,operation_id:crypto.randomUUID()}});
            await open(selected.manifest.id);setNotice("Revision accepted. No application files were changed.");
          })}>Accept revision</button>
          <form onSubmit={e=>{e.preventDefault();void run(async()=>{
            setProposal(null);
            setProposal(await call<PreparedHandoff>("draft_implementation",{request:{artifact_id:selected.manifest.id,revision:selected.manifest.revision,
              target_files:targets.split(/\r?\n/).map(path=>path.trim()).filter(Boolean),operation_id:crypto.randomUUID()}}));
          });}}><label>Implementation files, one relative path per line<textarea required value={targets} maxLength={8192} onChange={e=>{setTargets(e.target.value);setProposal(null);}}/></label>
            <p>Use clean UI source files in this coding session’s isolated Git worktree. Review the proposal before approving application changes.</p>
            <button disabled={busy||!selected.accepted}>Prepare implementation proposal</button></form>
          {proposal&&<section aria-label="Implementation approval"><h3>Review implementation</h3><p>{proposal.proposal.brief}</p>
            <pre tabIndex={0}>{proposal.proposal.request.patch}</pre><ul>{proposal.proposal.request.verification.map(check=><li key={check}>{check}</li>)}</ul>
            <details><summary>Approval identity</summary><code>{proposal.proposal_hash}</code></details>
            <button disabled={busy} onClick={()=>void run(async()=>{
              const result=await call<{implementation:string;transaction:{id:string}}>("apply",{request:{proposal_id:proposal.proposal.request.operation_id,
                proposal_hash:proposal.proposal_hash,operation_id:crypto.randomUUID()}});
              setProposal(null);setNotice("Patch applied in transaction "+result.transaction.id+". Target build, tests and browser verification remain required in the coding session.");
            })}>Approve and apply this patch</button></section>}
          <form onSubmit={e=>{e.preventDefault();void run(async()=>{
            const [width,height]=viewport.split("×").map(Number);
            const result=await call<ExportReceipt>("export",{request:{artifact_id:selected.manifest.id,revision:selected.manifest.revision,format:exportFormat,destination,
              artboard_id:exportFormat==="source"?null:board?.id,viewport:exportFormat==="png"?{width,height}:null,operation_id:crypto.randomUUID()}});
            setNotice((result.complete?"Export complete: ":"Export incomplete: ")+result.paths.join(", ")+(result.warning?" · "+result.warning:""));
          });}}><label>Export format<select value={exportFormat} onChange={e=>setExportFormat(e.target.value as ExportFormat)}><option value="source">Source bundle</option><option value="png">PNG capture</option><option value="html">Executable HTML</option></select></label>
            <label>New absolute destination directory<input value={destination} required onChange={e=>setDestination(e.target.value)}/></label>
            <button disabled={busy||!selected.revision}>Export revision</button></form>
        </>}
      </aside>
    </div>
  </main>;
}
createRoot(document.getElementById("root")!).render(<App/>);
