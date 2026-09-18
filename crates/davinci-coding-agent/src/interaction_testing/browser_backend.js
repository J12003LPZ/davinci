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
function createBrowserBackend(playwright) {
  let engine;
  let closing;
  let pending = 0;
  const sessions = new Set();
  const opening = new Set();
  function start() {
    if (!engine) {
      engine = playwright.chromium.launch({headless: true, timeout: 15000, args: [
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
        viewport.width < 128 || viewport.width > 1920 || viewport.height < 128 || viewport.height > 1080) {
      throw new Error('Browser viewport is outside bounds');
    }
    if (pending + sessions.size >= 8) throw new Error('Browser context limit');
    pending++;
    let context;
    let proxy;
    try {
      proxy = await createOriginProxy(options.origins);
      const origins = new Set(options.origins);
      if (closing || signal?.aborted) throw new Error('Browser open cancelled');
      const browser = await start();
      if (closing || signal?.aborted) throw new Error('Browser open cancelled');
      context = await browser.newContext({viewport, serviceWorkers: 'block', acceptDownloads: false,
        proxy: {server: proxy.serverUrl, bypass: '<-loopback>'}});
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
        if (allows(route.request().url(), origins)) await route.continue();
        else {record({kind: 'network', url: safeUrl(route.request().url()), status: 403}); await route.abort('accessdenied');}
      });
      await context.routeWebSocket('**/*', async route => {
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
        fields(command, ['action', 'url', 'selector', 'text', 'value']);
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
