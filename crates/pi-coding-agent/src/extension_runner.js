"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const Module = require("module");
const { pathToFileURL } = require("url");

const VIRTUAL_SPECIFIERS = [
	"@earendil-works/pi-agent-core",
	"@earendil-works/pi-tui",
	"@earendil-works/pi-ai",
	"@earendil-works/pi-ai/compat",
	"@earendil-works/pi-ai/oauth",
	"@earendil-works/pi-ai/providers/all",
	"@earendil-works/pi-coding-agent",
	"@mariozechner/pi-agent-core",
	"@mariozechner/pi-tui",
	"@mariozechner/pi-ai",
	"@mariozechner/pi-ai/compat",
	"@mariozechner/pi-ai/oauth",
	"@mariozechner/pi-ai/providers/all",
	"@mariozechner/pi-coding-agent",
	"typebox",
	"typebox/compile",
	"typebox/value",
	"@sinclair/typebox",
	"@sinclair/typebox/compile",
	"@sinclair/typebox/value",
];

const VIRTUAL_FILES = {};
const virtualRoot = path.join(os.tmpdir(), "pi-virtual-modules");
fs.mkdirSync(virtualRoot, { recursive: true });
for (const name of VIRTUAL_SPECIFIERS) {
	const file = path.join(virtualRoot, `${name.replace(/[@/]/g, "_")}.js`);
	const source = [
		"const exported = { __esModule: true, __piVirtual: " + JSON.stringify(name) + ", name: " + JSON.stringify(name) + ', version: "0.84.4" };',
		"exported.default = exported;",
		"module.exports = exported;",
	].join("\n");
	fs.writeFileSync(file, source);
	VIRTUAL_FILES[name] = file;
}

const originalResolveFilename = Module._resolveFilename;
Module._resolveFilename = function (request, parent, isMain, options) {
	if (Object.prototype.hasOwnProperty.call(VIRTUAL_FILES, request)) {
		return VIRTUAL_FILES[request];
	}
	return originalResolveFilename.call(this, request, parent, isMain, options);
};

function compileTypeScript(source, filename) {
	try {
		const ts = require("typescript");
		return ts.transpileModule(source, {
			fileName: filename,
			compilerOptions: {
				module: ts.ModuleKind.CommonJS,
				target: ts.ScriptTarget.ES2020,
				esModuleInterop: true,
				skipLibCheck: true,
			},
		}).outputText;
	} catch (_error) {
		return stripTypeScript(source);
	}
}

function stripTypeScript(source) {
	let out = source.replace(/^\uFEFF/, "");
	out = out.replace(/import\s+type\s+[\s\S]*?from\s+['"][^'"]+['"]\s*;?/g, "");
	out = out.replace(/export\s+type\s+[\s\S]*?;/g, "");
	out = out.replace(
		/import\s+\*\s+as\s+(\w+)\s+from\s+['"]([^'"]+)['"]\s*;?/g,
		"const $1 = require('$2');",
	);
	out = out.replace(
		/import\s+(\{[^}]*\})\s+from\s+['"]([^'"]+)['"]\s*;?/g,
		"const $1 = require('$2');",
	);
	out = out.replace(
		/import\s+(\w+)\s+from\s+['"]([^'"]+)['"]\s*;?/g,
		"const $1 = require('$2').default || require('$2');",
	);
	out = out.replace(/export\s+default\s+/g, "module.exports = ");
	return out;
}

function registerTypeScriptExtension(ext) {
	require.extensions[ext] = function compileTs(module, filename) {
		const source = fs.readFileSync(filename, "utf8");
		module._compile(compileTypeScript(source, filename), filename);
	};
}

for (const ext of [".ts", ".tsx", ".mts", ".cts"]) {
	registerTypeScriptExtension(ext);
}

async function loadWithJiti(modPath) {
	let createJiti;
	try {
		({ createJiti } = require("jiti"));
	} catch (_error) {
		return undefined;
	}
	if (typeof createJiti !== "function") {
		return undefined;
	}
	try {
		const jiti = createJiti(__filename, {
			moduleCache: false,
			interopDefault: true,
		});
		return await jiti.import(modPath);
	} catch (_error) {
		return undefined;
	}
}

function loadCompiledTypeScript(modPath) {
	const source = fs.readFileSync(modPath, "utf8");
	const compiled = compileTypeScript(source, modPath);
	const tmp = path.join(
		path.dirname(modPath),
		`.pi-compiled-${path.basename(modPath)}.cjs`,
	);
	fs.writeFileSync(tmp, compiled);
	try {
		delete require.cache[require.resolve(tmp)];
		return require(tmp);
	} finally {
		try {
			fs.unlinkSync(tmp);
		} catch (_error) {
			// best-effort cleanup
		}
	}
}

async function loadModule(modPath) {
	const ext = path.extname(modPath).toLowerCase();
	if ([".ts", ".tsx", ".mts", ".cts"].includes(ext)) {
		const fromJiti = await loadWithJiti(modPath);
		if (fromJiti !== undefined) {
			return fromJiti;
		}
		return loadCompiledTypeScript(modPath);
	}
	try {
		return require(modPath);
	} catch (error) {
		if (error && (error.code === "ERR_REQUIRE_ESM" || String(error).includes("Cannot use import"))) {
			const ns = await import(pathToFileURL(modPath).href);
			return ns.default !== undefined ? ns.default : ns;
		}
		throw error;
	}
}

async function main() {
	const extPath = path.resolve(process.argv[2] || "");
	const op = process.argv[3] || "load";
	let input = "";
	if (!process.stdin.isTTY) {
		input = fs.readFileSync(0, "utf8");
	}
	const payload = input.trim() ? JSON.parse(input) : {};
	const mod = await loadModule(extPath);
	const factory = typeof mod === "function" ? mod : mod.default || mod.factory;
	const recorded = {
		handlers: {},
		tools: [],
		commands: [],
		flags: [],
		shortcuts: [],
		shortcutHandlers: {},
		messageRenderers: {},
		markdownTransformers: [],
		entryRenderers: {},
	};
	const pi = {
		on(event, handler) {
			if (!recorded.handlers[event]) recorded.handlers[event] = [];
			recorded.handlers[event].push(handler);
		},
		registerTool(tool) {
			recorded.tools.push({ name: tool.name, description: tool.description || "" });
		},
		registerCommand(name, options) {
			recorded.commands.push({ name, description: (options && options.description) || "" });
		},
		registerFlag(name) {
			recorded.flags.push(name);
		},
		registerShortcut(key, options) {
			recorded.shortcuts.push(key);
			if (options && typeof options.handler === "function") {
				recorded.shortcutHandlers[String(key).toLowerCase()] = options.handler;
			}
		},
		registerMessageRenderer(customType, renderer) {
			recorded.messageRenderers[customType] = renderer;
		},
		registerMarkdownTransformer(transformer) {
			recorded.markdownTransformers.push(transformer);
		},
		registerEntryRenderer(customType, renderer) {
			recorded.entryRenderers[customType] = renderer;
		},
		sendMessage() {},
		sendUserMessage() {},
		appendEntry() {},
		setSessionName() {},
		getSessionName() {
			return undefined;
		},
		setLabel() {},
		exec() {
			return Promise.resolve({ code: 0, stdout: "", stderr: "" });
		},
		getActiveTools() {
			return [];
		},
		getAllTools() {
			return [];
		},
		setActiveTools() {},
		getCommands() {
			return [];
		},
		setModel() {
			return Promise.resolve(false);
		},
		getThinkingLevel() {
			return "off";
		},
		setThinkingLevel() {},
		registerProvider() {},
		unregisterProvider() {},
		getFlag() {
			return undefined;
		},
		events: {
			emit() {},
			on() {
				return () => {};
			},
		},
	};
	if (typeof factory === "function") {
		await factory(pi);
	}
	let result = null;
	if (op === "emit") {
		const eventName = payload.type || payload.event;
		for (const handler of recorded.handlers[eventName] || []) {
			result = await handler(payload, payload.ctx || {});
			if (result && (result.block || result.cancel || result.action === "handled")) {
				break;
			}
		}
	} else if (op === "renderMessage") {
		const renderer = recorded.messageRenderers[payload.customType];
		const theme = {
			fg(_role, text) {
				return text;
			},
			bg(_role, text) {
				return text;
			},
			bold(text) {
				return text;
			},
		};
		if (typeof renderer === "function") {
			const component = renderer(
				payload.message || {
					role: "custom",
					customType: payload.customType,
					content: payload.content || "",
				},
				payload.options || { expanded: false, outputPad: 1 },
				theme,
			);
			if (component == null) {
				result = { lines: null };
			} else if (typeof component.render === "function") {
				result = { lines: component.render(payload.width || 80) };
			} else if (typeof component === "string") {
				result = { lines: [component] };
			} else if (Array.isArray(component)) {
				result = { lines: component };
			} else if (component && Array.isArray(component.lines)) {
				result = { lines: component.lines };
			}
		}
	} else if (op === "renderEntry") {
		const renderer = recorded.entryRenderers[payload.customType];
		const theme = {
			fg(_role, text) {
				return text;
			},
			bg(_role, text) {
				return text;
			},
			bold(text) {
				return text;
			},
		};
		if (typeof renderer === "function") {
			const component = renderer(
				payload.entry || {
					type: "custom",
					customType: payload.customType,
					data: payload.data || {},
				},
				payload.options || { expanded: false },
				theme,
			);
			if (component == null) {
				result = { lines: null };
			} else if (typeof component.render === "function") {
				result = { lines: component.render(payload.width || 80) };
			} else if (typeof component === "string") {
				result = { lines: [component] };
			} else if (Array.isArray(component)) {
				result = { lines: component };
			} else if (component && Array.isArray(component.lines)) {
				result = { lines: component.lines };
			}
		}
	} else if (op === "transformMarkdown") {
		let text = payload.markdown || payload.text || "";
		const context = payload.context || {
			messageType: "assistant",
			isStreaming: false,
			availableWidth: payload.width || 80,
		};
		for (const transformer of recorded.markdownTransformers) {
			if (typeof transformer === "function") {
				const next = transformer(text, context);
				if (typeof next === "string") {
					text = next;
				}
			}
		}
		result = { markdown: text };
	} else if (op === "shortcut") {
		const key = String(payload.key || "").toLowerCase();
		const handler = recorded.shortcutHandlers[key];
		if (typeof handler === "function") {
			result = await handler(payload.ctx || {});
		}
	}
	process.stdout.write(
		JSON.stringify({
			ok: true,
			handlers: Object.keys(recorded.handlers),
			tools: recorded.tools,
			commands: recorded.commands,
			flags: recorded.flags,
			shortcuts: recorded.shortcuts,
			messageRenderers: Object.keys(recorded.messageRenderers),
			entryRenderers: Object.keys(recorded.entryRenderers),
			markdownTransformers: recorded.markdownTransformers.length,
			result,
		}),
	);
}

main().catch((error) => {
	process.stdout.write(
		JSON.stringify({
			ok: false,
			error: error && error.message ? error.message : String(error),
		}),
	);
	process.exitCode = 1;
});
