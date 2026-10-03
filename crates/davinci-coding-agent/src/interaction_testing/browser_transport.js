'use strict';

// Trusted host transport, not a model permission/resource registry. The Rust
// owner must authorize each command and provide artifact retention separately.
const MAX_FRAME = 16 * 1024;
const MAX_RESPONSE = 64 * 1024;
const MAX_PENDING = 16;
const ACTIONS = new Set(['navigate', 'click', 'type', 'select', 'snapshot',
  'accessibility', 'console', 'network', 'screenshot']);

function fields(value, keys) {
  if (!value || typeof value !== 'object' || Array.isArray(value) ||
      Object.keys(value).some(key => !keys.includes(key))) throw new Error('Invalid fields');
}
function id(value) {
  if (!Number.isSafeInteger(value) || value < 1) throw new Error('Invalid ID');
  return value;
}
function validate(request) {
  id(request?.id);
  switch (request.op) {
    case 'open': fields(request, ['id', 'op', 'options']); fields(request.options, ['origins', 'viewport']); break;
    case 'execute':
      fields(request, ['id', 'op', 'resource', 'command']); id(request.resource);
      fields(request.command, ['action', 'url', 'selector', 'text', 'value']);
      if (!ACTIONS.has(request.command.action)) throw new Error('Invalid action');
      break;
    case 'close': fields(request, ['id', 'op', 'resource']); id(request.resource); break;
    case 'cancel':
      fields(request, ['id', 'op', 'target']); id(request.target);
      if (request.target >= request.id) throw new Error('Invalid cancellation target');
      break;
    case 'shutdown': fields(request, ['id', 'op']); break;
    default: throw new Error('Invalid operation');
  }
}

function createBrowserTransport({input, output, backend, artifact}) {
  let frame = Buffer.alloc(0);
  let highWater = 0;
  let stopping;
  let failed = false;
  let finish;
  const done = new Promise(resolve => {finish = resolve;});
  const resources = new Map();
  const opening = new Map();
  const active = new Map();
  const pending = new Set();
  function respond(value) {
    let serialized = JSON.stringify(value);
    if (Buffer.byteLength(serialized) + 1 > MAX_RESPONSE) {
      serialized = JSON.stringify({id: value.id, ok: false, error: 'Browser response limit'});
    }
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        output.destroy(new Error('Browser output deadline'));
        reject(new Error('Browser output deadline'));
      }, 5000);
      try {
        output.write(serialized + '\n', error => {
          clearTimeout(timeout); if (error) reject(error); else resolve();
        });
      } catch (error) {clearTimeout(timeout); reject(error);}
    });
  }
  async function stop(protocolFailure = false, requestId) {
    failed ||= protocolFailure;
    if (stopping) return stopping;
    // Remove input authority before awaiting cleanup. In-flight close/shutdown
    // is independent of the per-context action queue, allowing interruption.
    input.off('data', receive); input.pause();
    stopping = (async () => {
      for (const controller of opening.values()) controller.abort();
      const results = await Promise.allSettled([...resources.values()].map(async resource => {
        resource.controller.abort(); await resource.session.close();
      }));
      resources.clear();
      try {await backend.close();} catch {failed = true;}
      if (results.some(result => result.status === 'rejected')) failed = true;
      await Promise.allSettled([...pending]);
      try {
        if (protocolFailure) await respond({id: null, ok: false, error: 'Browser transport protocol failure'});
        else if (requestId !== undefined) await respond(failed ?
          {id: requestId, ok: false, error: 'Browser cleanup failed'} : {id: requestId, ok: true, result: {closed: true}});
      } catch {failed = true;}
      finish({failed});
    })();
    return stopping;
  }
  async function dispatch(request) {
    let result;
    try {
      if (request.op === 'cancel') {
        const operation = active.get(request.target);
        if (operation && !['open', 'execute'].includes(operation.request.op)) throw new Error('Invalid cancellation target');
        const controller = opening.get(request.target);
        controller?.abort();
        // An open response can race the host cancellation flag. Its correlation
        // ID remains the resource ID, so that context can still be reclaimed.
        const resourceId = operation?.request.op === 'execute' ? operation.request.resource : request.target;
        const resource = resources.get(resourceId);
        if (resource) {
          resources.delete(resourceId); resource.controller.abort();
          try {await resource.session.close();} catch (error) {failed = true; throw error;}
        }
        if (operation) await operation.promise;
        result = {cancelled: Boolean(controller || resource || operation)};
      } else if (request.op === 'open') {
        const controller = new AbortController();
        opening.set(request.id, controller);
        let session;
        try {session = await backend.open(request.options, controller.signal);}
        finally {opening.delete(request.id);}
        if (stopping || controller.signal.aborted) {await session.close(); throw new Error('Cancelled');}
        resources.set(request.id, {session, controller});
        result = {resource: request.id, browserVersion: session.browserVersion};
      } else {
        const resource = resources.get(request.resource);
        if (!resource) throw new Error('Unknown resource');
        if (request.op === 'close') {
          resources.delete(request.resource);
          resource.controller.abort();
          try {await resource.session.close();} catch (error) {failed = true; throw error;}
          result = {closed: true};
        } else {
          if (request.command.action === 'screenshot' && !artifact) throw new Error('No artifact sink');
          result = await resource.session.execute(request.command);
          if (request.command.action === 'screenshot') {
            if (!Buffer.isBuffer(result.bytes) || result.bytes.length > 4 * 1024 * 1024 || result.mediaType !== 'image/png') {
              throw new Error('Invalid screenshot');
            }
            result = await artifact(request.resource, result);
            fields(result, ['artifact', 'size', 'mediaType']);
            if (typeof result.artifact !== 'string' || result.artifact.length > 1024 ||
                !Number.isSafeInteger(result.size) || result.size < 0 || result.size > 4 * 1024 * 1024 ||
                result.mediaType !== 'image/png') throw new Error('Invalid artifact reference');
          }
        }
      }
      await respond({id: request.id, ok: true, result});
    } catch {
      // Backend exceptions can contain page contents, credentials or URLs.
      await respond({id: request.id, ok: false, error: 'Browser request failed'});
    }
  }
  function accept(bytes) {
    const request = JSON.parse(new TextDecoder('utf-8', {fatal: true}).decode(bytes));
    validate(request);
    if (request.id <= highWater) throw new Error('Replayed request');
    highWater = request.id;
    if (request.op === 'shutdown') {void stop(false, request.id); return;}
    // Reserve bounded control capacity so saturation cannot prevent cancellation.
    if (pending.size >= (request.op === 'cancel' ? MAX_PENDING * 2 : MAX_PENDING)) throw new Error('Pending limit');
    const record = {request}; active.set(request.id, record);
    const operation = dispatch(request);
    record.promise = operation;
    pending.add(operation);
    operation.finally(() => {pending.delete(operation); active.delete(request.id);}).catch(() => {void stop(true);});
  }
  function receive(chunk) {
    try {
      if (!Buffer.isBuffer(chunk)) throw new Error('Byte stream required');
      let offset = 0;
      while (offset < chunk.length && !stopping) {
        const newline = chunk.indexOf(10, offset);
        const end = newline < 0 ? chunk.length : newline;
        if (frame.length + end - offset > MAX_FRAME) throw new Error('Frame limit');
        frame = Buffer.concat([frame, chunk.subarray(offset, end)]);
        if (newline < 0) break;
        const bytes = frame; frame = Buffer.alloc(0);
        accept(bytes); offset = newline + 1;
      }
    } catch {void stop(true);}
  }
  input.on('data', receive);
  input.on('end', () => {void stop(frame.length !== 0);});
  input.on('error', () => {void stop(true);});
  output.on('error', () => {void stop(true);});
  return {done, close: () => stop()};
}

module.exports = {createBrowserTransport};
