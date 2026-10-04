let capability: string | null = null;
export class DesignFailure extends Error {}
export async function pair(): Promise<void> {
  const fragment = new URLSearchParams(location.hash.slice(1));
  const token = fragment.get("pair");
  history.replaceState(null, "", location.pathname);
  if (!token || !/^[a-f0-9]{64}$/.test(token)) throw new DesignFailure("Pairing link is missing or expired. Open the workspace again from DaVinci.");
  const response = await fetch("/pair", { method:"POST", headers:{"content-type":"application/json"}, body:JSON.stringify({token}), credentials:"omit", cache:"no-store" });
  if (!response.ok) throw new DesignFailure("This pairing link has expired or was already used. Open a new workspace link.");
  capability = (await response.json()).capability;
}
async function request(operation: string, payload: object, signal?: AbortSignal): Promise<{result?:unknown;job?:string}> {
  if (!capability) throw new DesignFailure("Workspace is not paired.");
  const response = await fetch("/api", {method:"POST", credentials:"omit", cache:"no-store", signal,
    headers:{"content-type":"application/json",authorization:"Bearer "+capability},
    body:JSON.stringify({version:1,id:crypto.randomUUID(),operation,payload})});
  if (response.status === 403) {capability=null;throw new DesignFailure("Workspace access expired. Reopen it from DaVinci.");}
  const value = await response.json();
  if (!response.ok || value.error) throw new DesignFailure(value.message || (typeof value.error==="string"?value.error:value.error?.message) || "The request failed.");
  if (value.result?.error) throw new DesignFailure(value.result.error.message || value.result.error.code || "Operation denied.");
  return value;
}
export async function call<T>(operation: string, payload: object, signal?: AbortSignal): Promise<T> {
  signal?.throwIfAborted();
  // Receive the job identity even if cancelled during dispatch so cancellation
  // reaches the native operation instead of orphaning it until its deadline.
  const longOperation=["generate","verify","render","interact","draft_implementation","verify_implementation"].includes(operation);
  const value=await request(operation,payload,longOperation?undefined:signal);
  if(!value.job)return value.result as T;
  const job=value.job;
  const cancel=()=>{void request("job_cancel",{job_id:job}).catch(()=>{});};
  signal?.addEventListener("abort",cancel,{once:true});
  try {
    if(signal?.aborted){cancel();signal.throwIfAborted();}
    for(;;){
      signal?.throwIfAborted();
      const polled=await request("job_poll",{job_id:job},signal);
      const state=polled.result as {status:string;result:T;error?:string};
      if(state.status==="complete")return state.result;
      if(state.status==="failed"||state.status==="cancelled")throw new DesignFailure(state.error||"Operation cancelled. Last committed source is retained.");
      if(state.status!=="running")throw new DesignFailure("Invalid operation state");
      await new Promise(resolve=>setTimeout(resolve,500));
    }
  } finally {signal?.removeEventListener("abort",cancel);}
}
export async function captureBlob(payload:object,expectedHash:string,signal:AbortSignal):Promise<Blob>{
  let offset=0,total=0;
  const chunks:Uint8Array<ArrayBuffer>[]=[];
  do {
    const result=await call<{bytes:string;next:number;total:number;sha256:string}>("capture",{...payload,offset},signal);
    const bytes=Uint8Array.from(atob(result.bytes),char=>char.charCodeAt(0));
    if(!Number.isSafeInteger(result.total)||result.total<1||result.total>4*1024*1024||
      result.next!==offset+bytes.length||result.next>result.total||bytes.length===0||
      (offset>0&&result.total!==total)||result.sha256!==expectedHash)throw new DesignFailure("Capture transfer changed.");
    total=result.total;offset=result.next;chunks.push(bytes);
  } while(offset<total);
  const blob=new Blob(chunks,{type:"image/png"});
  const actual=[...new Uint8Array(await crypto.subtle.digest("SHA-256",await blob.arrayBuffer()))].map(v=>v.toString(16).padStart(2,"0")).join("");
  if(actual!==expectedHash)throw new DesignFailure("Capture digest does not match this revision.");
  return blob;
}
