'use strict';

const fs = require('node:fs');
const path = require('node:path');
const {createOriginProxy} = require('./browser_network.js');

const MAX_TEXT_BYTES = 48 * 1024;
const MAX_EVENTS = 128;

function fields(value, names) {
  if (!value || typeof value !== 'object' || Array.isArray(value) ||
      Object.keys(value).some(name => !names.includes(name))) throw new Error('Invalid browser fields');
}
function text(value, limit = 1024) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > limit) throw new Error('Invalid browser text');
  return value;
}
function bounded(value) {return text(value, MAX_TEXT_BYTES);}
function safeUrl(value) {
  try {const url = new URL(value); url.search = ''; url.hash = ''; url.username = ''; url.password = ''; return url.href.slice(0, 1024);}
  catch {return '[invalid URL]';}
}
function allows(value, origins, websocket = false) {
  try {
    const url = new URL(value);
    if (websocket && url.protocol === 'ws:') url.protocol = 'http:';
    if (websocket && url.protocol === 'wss:') url.protocol = 'https:';
    return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password && origins.has(url.origin);
  } catch {return false;}
}

// Host-only configuration, never populated from model tool arguments. Resolving
// a package relative to a project would execute repository-controlled code.
function loadTrustedPlaywright(config) {
  fields(config, ['packagePath', 'version', 'workspace']);
  if (!path.isAbsolute(config.packagePath) || !path.isAbsolute(config.workspace)) {
    throw new Error('Trusted browser paths must be absolute');
  }
  // Rust canonical paths use Windows' extended prefix; the native resolver
  // handles it without the JavaScript resolver's drive-relative lstat fallback.
  const packagePath = fs.realpathSync.native(config.packagePath);
  const workspace = fs.realpathSync.native(config.workspace);
  const relative = path.relative(workspace, packagePath);
  if (relative === '' || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative))) {
    throw new Error('Trusted browser package must be outside the workspace');
  }
  const manifest = JSON.parse(fs.readFileSync(path.join(packagePath, 'package.json'), 'utf8'));
  if (!['playwright', 'playwright-core'].includes(manifest.name) || manifest.version !== text(config.version, 64)) {
    throw new Error('Trusted browser package version mismatch');
  }
  return require(packagePath);
}

// The Rust host retains resource ownership and checks current authorization
// before every call. These session objects are backend resources, not a second
// permission registry or model-facing handle database.
function createBrowserBackend(playwright, design) {
  const rasterAssets = new Map();
  // Only the confined Rust launch can supply this private, immutable bundle.
  if (design) {
    fields(design, ['files', 'assets', 'theme', 'reducedMotion', 'executable']);
    if (!design.files || Object.keys(design.files).length > 64 ||
        Buffer.byteLength(JSON.stringify(design.files)) > 12 * 1024 * 1024 ||
        !['light','dark'].includes(design.theme) || typeof design.reducedMotion !== 'boolean' ||
        !path.isAbsolute(design.executable)) throw new Error('Invalid design');
    for (const [name,value] of Object.entries(design.files)) {
      if (!name || name.startsWith('/') || name.split('/').some(part => !/^[a-zA-Z0-9._@-]+$/.test(part) || ['.','..'].includes(part)) ||
          typeof value !== 'string') throw new Error('Invalid design file');
    }
    if (design.assets !== undefined && (!design.assets || Array.isArray(design.assets) || typeof design.assets !== 'object')) throw new Error('Invalid design assets');
    const assets = Object.entries(design.assets || {});
    if (assets.length > 20) throw new Error('Design asset count limit');
    const names = new Set(Object.keys(design.files).map(name => name.toLowerCase()));
    let total = 0;
    for (const [name, asset] of assets) {
      fields(asset, ['mediaType', 'base64']);
      if (!name || name.startsWith('/') || name.split('/').some(part => !/^[a-zA-Z0-9._@-]+$/.test(part) || ['.','..'].includes(part)) ||
          names.has(name.toLowerCase()) || !['image/png','image/jpeg','image/webp'].includes(asset.mediaType) ||
          typeof asset.base64 !== 'string' || asset.base64.length > 4 * Math.ceil(10 * 1024 * 1024 / 3) ||
          asset.base64.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(asset.base64)) throw new Error('Invalid design asset');
      const size = asset.base64.length / 4 * 3 - (asset.base64.endsWith('==') ? 2 : asset.base64.endsWith('=') ? 1 : 0);
      total += size;
      if (size > 10 * 1024 * 1024 || total > 100 * 1024 * 1024) throw new Error('Design asset byte limit');
      names.add(name.toLowerCase());
      const bytes = Buffer.from(asset.base64, 'base64');
      if (bytes.toString('base64') !== asset.base64) throw new Error('Noncanonical design asset');
      rasterAssets.set(name, {mediaType:asset.mediaType, bytes});
    }
  }
  let engine;
  let closing;
  let pending = 0;
  const sessions = new Set();
  const opening = new Set();
  function start() {
    if (!engine) {
      engine = playwright.chromium.launch({headless: true, timeout: 15000, ...(design?{executablePath:design.executable}:{}), args: [
        '--disable-quic', '--disable-http2', '--force-webrtc-ip-handling-policy=disable_non_proxied_udp',
      ]}).catch(error => {engine = undefined; throw error;});
    }
    return engine;
  }
  async function open(options, signal) {
    fields(options, ['origins', 'viewport']);
    if (closing || signal?.aborted) throw new Error('Browser open cancelled');
    const viewport = options.viewport || {width: 1280, height: 720};
    fields(viewport, ['width', 'height']);
    if (!Number.isInteger(viewport.width) || !Number.isInteger(viewport.height) ||
        viewport.width < 128 || viewport.width > (design ? 4096 : 1920) || viewport.height < 128 ||
        viewport.height > (design ? 4096 : 1080) || viewport.width * viewport.height > 8000000) {
      throw new Error('Browser viewport is outside bounds');
    }
    if (pending + sessions.size >= 8) throw new Error('Browser context limit');
    pending++;
    let context;
    let proxy;
    try {
      if (design && (options.origins?.length !== 1 || options.origins[0] !== 'https://design.invalid')) throw new Error('Invalid design origin');
      proxy = design ? {metrics:()=>({denied:0}),close:async()=>{}} : await createOriginProxy(options.origins);
      const origins = new Set(options.origins);
      if (closing || signal?.aborted) throw new Error('Browser open cancelled');
      const browser = await start();
      if (closing || signal?.aborted) throw new Error('Browser open cancelled');
      context = await browser.newContext({viewport, serviceWorkers: 'block', acceptDownloads: false,
        ...(design ? {colorScheme:design.theme, reducedMotion:design.reducedMotion?'reduce':'no-preference', permissions:[]} :
          {proxy: {server: proxy.serverUrl, bypass: '<-loopback>'}})});
      if (closing || signal?.aborted) throw new Error('Browser open cancelled');
      const events = [];
      let eventBytes = 0;
      let omitted = false;
      let closed;
      let queue = Promise.resolve();
      let pages = 0;
      function record(event) {
        const bytes = Buffer.byteLength(JSON.stringify(event)) + 1;
        if (events.length >= MAX_EVENTS || eventBytes + bytes > MAX_TEXT_BYTES) {omitted = true; return;}
        eventBytes += bytes;
        events.push(event);
      }
      function attach(page) {
        if (design && pages > 0) { record({kind:'error',message:'Design popup blocked'}); page.close().catch(()=>{}); return; }
        if (++pages > 8) {record({kind: 'error', message: 'Browser page limit'}); page.close().catch(() => {}); return;}
        page.setDefaultTimeout(5000);
        page.on('console', message => {
          if (message.type() === 'error') record({kind: 'console', message: message.text().slice(0, 1024)});
        });
        page.on('pageerror', error => record({kind: 'console', message: String(error.message).slice(0, 1024)}));
        page.on('download', download => {record({kind: 'error', message: 'Browser download blocked'}); download.cancel().catch(() => {});});
      }
      context.on('page', attach);
      context.on('requestfailed', request => record({kind: 'network', url: safeUrl(request.url()), status: 0,
        message: String(request.failure()?.errorText || 'request failed').slice(0, 1024)}));
      context.on('response', response => {
        if (response.status() >= 400) record({kind: 'network', url: safeUrl(response.url()), status: response.status()});
      });
      await context.route('**/*', async route => {
        if (design) {
          const url = new URL(route.request().url());
          const filename = url.pathname.slice(1);
          if (url.origin !== 'https://design.invalid' || url.username || url.password || url.search ||
              !(Object.hasOwn(design.files, filename) || rasterAssets.has(filename)) || route.request().method() !== 'GET') {
            record({kind:'network',url:safeUrl(url.href),status:403}); await route.abort('accessdenied'); return;
          }
          const types = {'.html':'text/html; charset=utf-8','.js':'text/javascript; charset=utf-8',
            '.css':'text/css; charset=utf-8','.svg':'image/svg+xml','.json':'application/json'};
          const asset = rasterAssets.get(filename);
          await route.fulfill({status:200,contentType:asset?.mediaType || types[path.extname(filename)]||'text/plain',
            body:asset ? asset.bytes : design.files[filename],headers:{
              'Content-Security-Policy':"default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'none'; worker-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'",
              'X-Content-Type-Options':'nosniff','Permissions-Policy':'camera=(), microphone=(), geolocation=()',
            }});
          return;
        }
        if (allows(route.request().url(), origins)) await route.continue();
        else {record({kind: 'network', url: safeUrl(route.request().url()), status: 403}); await route.abort('accessdenied');}
      });
      await context.routeWebSocket('**/*', async route => {
        if (design) { record({kind:'network',url:safeUrl(route.url()),status:403}); await route.close({code:1008,reason:'Design network denied'}); return; }
        if (allows(route.url(), origins, true)) route.connectToServer();
        else {record({kind: 'network', url: safeUrl(route.url()), status: 403}); await route.close({code: 1008, reason: 'Origin policy'});}
      });
      const page = await context.newPage();
      function locator(selector) {
        fields(selector, ['kind', 'value', 'role', 'name']);
        if (selector.kind === 'test_id') {
          fields(selector, ['kind', 'value']); return page.getByTestId(text(selector.value));
        }
        if (selector.kind === 'role') {
          fields(selector, ['kind', 'role', 'name']);
          return page.getByRole(text(selector.role, 64), {name: text(selector.name), exact: true});
        }
        if (selector.kind === 'label') {
          fields(selector, ['kind', 'value']); return page.getByLabel(text(selector.value), {exact: true});
        }
        throw new Error('Unsupported browser selector');
      }
      async function action(command) {
        if (closed || signal?.aborted) throw new Error('Browser session closed');
        fields(command, ['action', 'url', 'selector', 'text', 'value', 'key']);
        if (Buffer.byteLength(JSON.stringify(command)) > 64 * 1024) throw new Error('Browser command limit');
        switch (command.action) {
          case 'navigate': {
            fields(command, ['action', 'url']);
            if (!allows(text(command.url, 8192), origins)) throw new Error('Browser origin denied');
            const response = await page.goto(command.url, {waitUntil: 'load', timeout: 5000});
            return {url: safeUrl(page.url()), status: response?.status() ?? 0};
          }
          case 'click':
            fields(command, ['action', 'selector']); await locator(command.selector).click(); return {done: true};
          case 'type':
            fields(command, ['action', 'selector', 'text']); await locator(command.selector).fill(text(command.text, 4096)); return {done: true};
          case 'select':
            fields(command, ['action', 'selector', 'value']); await locator(command.selector).selectOption(text(command.value)); return {done: true};
          case 'key': {
            fields(command, ['action', 'key']);
            if (!design || !['Tab','Shift+Tab','Enter','Escape','Space','ArrowUp','ArrowDown','ArrowLeft','ArrowRight'].includes(command.key)) throw new Error('Unsupported prototype key');
            await page.keyboard.press(command.key); return {done:true};
          }
          case 'expect_text': {
            fields(command, ['action', 'text']);
            if (!design) throw new Error('Prototype assertion unavailable');
            const expected=text(command.text,512);
            // The assertion inspects rendered text only, never evaluates caller code.
            await page.getByText(expected,{exact:false}).first().waitFor({state:'visible',timeout:5000});
            return {matched:true};
          }
          case 'snapshot':
            fields(command, ['action']); return {html: bounded(await page.content())};
          case 'accessibility':
            fields(command, ['action']); return {tree: bounded(await page.locator('body').ariaSnapshot({timeout: 5000}))};
          case 'console':
            fields(command, ['action']); return {events: events.filter(event => event.kind === 'console'), omitted};
          case 'network':
            fields(command, ['action']); return {events: events.filter(event => event.kind === 'network'), omitted,
              policy: proxy.metrics()};
          case 'screenshot': {
            fields(command, ['action']);
            const bytes = await page.screenshot({type: 'png', timeout: 5000});
            if (bytes.length > 4 * 1024 * 1024) throw new Error('Browser screenshot limit');
            // Host stores these bytes through the existing artifact tracker.
            return {bytes, mediaType: 'image/png'};
          }
          case 'design_geometry': {
            fields(command, ['action']);
            if (!design) throw new Error('Design geometry unavailable');
            const geometry = await page.evaluate(() => {
              const all = [...document.querySelectorAll('[data-design-node],button,a,input,select,textarea,img')];
              const nodes = all.slice(0,200).map(element => {
                const rectangle = element.getBoundingClientRect();
                const style = getComputedStyle(element);
                const interactive = element.matches('button,a,input,select,textarea');
                const name = element.getAttribute('aria-label') || element.getAttribute('alt') ||
                  (element.labels ? [...element.labels].map(label=>label.textContent).join(' ') : '') || element.textContent || '';
                return {id:(element.getAttribute('data-design-node')||'').slice(0,64),tag:element.tagName.toLowerCase(),
                  x:Math.round(rectangle.x),y:Math.round(rectangle.y),width:Math.round(rectangle.width),height:Math.round(rectangle.height),
                  visible:style.visibility!=='hidden' && style.display!=='none' && rectangle.width>0 && rectangle.height>0,
                  interactive,name:name.trim().slice(0,120),focusable:element.tabIndex>=0};
              });
              return {nodes,omitted:all.length>200,overflow:document.documentElement.scrollWidth>innerWidth+1,
                title:document.title.slice(0,120),lang:document.documentElement.lang.slice(0,32)};
            });
            return {...geometry,events:events.map(event=>({...event})),eventsOmitted:omitted};
          }
          default: throw new Error('Unsupported browser action');
        }
      }
      const session = {
        browserVersion: browser.version(),
        execute(command) {
          const next = queue.then(() => action(command));
          queue = next.catch(error => {record({kind: 'error', message: String(error.message).slice(0, 1024)});});
          return next;
        },
        evidence: () => ({events: events.map(event => ({...event})), omitted,
          healthy: events.length === 0 && !omitted && proxy.metrics().denied === 0}),
        close() {
          if (!closed) {
            signal?.removeEventListener('abort', cancel);
            closed = Promise.allSettled([context.close(), proxy.close()]).then(results => {
              sessions.delete(session);
              if (results.some(result => result.status === 'rejected')) throw new Error('Browser cleanup failed');
            });
          }
          return closed;
        },
      };
      function cancel() {session.close().catch(() => {});}
      signal?.addEventListener('abort', cancel, {once: true});
      if (closing || signal?.aborted) {await session.close(); throw new Error('Browser open cancelled');}
      sessions.add(session);
      return session;
    } catch (error) {
      await Promise.allSettled([context?.close(), proxy?.close()]);
      throw error;
    } finally {pending--;}
  }
  return {
    open(options, signal) {
      const result = open(options, signal);
      opening.add(result); result.finally(() => opening.delete(result)).catch(() => {});
      return result;
    },
    close() {
      if (!closing) {
        closing = (async () => {
          await Promise.allSettled([...opening]);
          const results = await Promise.allSettled([...sessions].map(session => session.close()));
          if (engine) await (await engine).close();
          if (results.some(result => result.status === 'rejected')) throw new Error('Browser shutdown failed');
        })();
      }
      return closing;
    },
  };
}

module.exports = {createBrowserBackend, loadTrustedPlaywright};
