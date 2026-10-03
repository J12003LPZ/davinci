import React, {useState} from "react";
import type {PrototypeAction,PrototypeSelector} from "./contracts.generated";

export function InteractionPanel({busy,count,onAction,onReset}:{busy:boolean;count:number;onAction:(action:PrototypeAction)=>void;onReset:()=>void}) {
  const [role,setRole]=useState("button"),[name,setName]=useState(""),[value,setValue]=useState(""),[action,setAction]=useState("click"),[key,setKey]=useState("Tab");
  const selector:PrototypeSelector={kind:"role",role,name};
  return <section aria-label="Prototype interaction"><h3>Interact with the prototype</h3>
    <p>Each action replays the sequence in a fresh confined browser. {count}/16 actions. These checks cover the prototype only.</p>
    <form onSubmit={event=>{event.preventDefault();onAction(action==="click"?{action:"click",selector}:action==="type"?{action:"type",selector,text:value}:action==="select"?{action:"select",selector,value}:{action:"expect_text",text:value});}}>
      <label>Action<select value={action} onChange={event=>setAction(event.target.value)}><option value="click">Click</option><option value="type">Fill field</option><option value="select">Select option</option><option value="expect_text">Check visible outcome</option></select></label>
      {action!=="expect_text"&&<><label>Element role<select value={role} onChange={event=>setRole(event.target.value)}>{["button","link","textbox","checkbox","radio","combobox","tab","menuitem","switch"].map(role=><option key={role}>{role}</option>)}</select></label>
        <label>Exact accessible name<input required maxLength={120} value={name} onChange={event=>setName(event.target.value)}/></label></>}
      {action!=="click"&&<label>{action==="expect_text"?"Expected visible text":"Value"}<input required maxLength={action==="type"?4096:action==="select"?120:512} value={value} onChange={event=>setValue(event.target.value)}/></label>}
      <button disabled={busy||count>=16}>Run action and capture</button>
    </form>
    <label>Keyboard key<select value={key} onChange={event=>setKey(event.target.value)}>{["Tab","Shift+Tab","Enter","Escape","Space","ArrowUp","ArrowDown","ArrowLeft","ArrowRight"].map(key=><option key={key}>{key}</option>)}</select></label>
    <button disabled={busy||count>=16} onClick={()=>onAction({action:"key",key})}>Press key and capture</button>
    <button disabled={busy||count===0} onClick={onReset}>Reset sequence</button>
  </section>;
}
