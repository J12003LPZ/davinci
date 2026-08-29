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

const TUI_VIRTUAL = `
class Container {
	constructor() { this.children = []; }
	addChild(component) { if (component) this.children.push(component); }
	removeChild(component) { this.children = this.children.filter((item) => item !== component); }
	clear() { this.children = []; }
	invalidate() { for (const child of this.children) { if (child && typeof child.invalidate === "function") child.invalidate(); } }
	render(width) {
		const lines = [];
		for (const child of this.children) {
			if (child && typeof child.render === "function") lines.push(...child.render(width));
		}
		return lines;
	}
	handleInput(data) {
		for (const child of this.children) {
			if (child && typeof child.handleInput === "function") child.handleInput(data);
		}
	}
}
class Text {
	constructor(value) { this.value = value == null ? "" : String(value); }
	setText(value) { this.value = value == null ? "" : String(value); }
	invalidate() {}
	render(width) {
		const text = String(this.value);
		if (text.length <= width) return [text];
		const lines = [];
		for (let i = 0; i < text.length; i += width) lines.push(text.slice(i, i + width));
		return lines;
	}
}
class Input {
	constructor() {
		this.value = "";
		this.focused = true;
		this.onSubmit = undefined;
		this.onEscape = undefined;
	}
	getValue() { return this.value; }
	setValue(value) { this.value = value == null ? "" : String(value); }
	invalidate() {}
	handleInput(data) {
		if (matchesKey(data, "escape")) { if (typeof this.onEscape === "function") this.onEscape(); return; }
		if (matchesKey(data, "enter")) { if (typeof this.onSubmit === "function") this.onSubmit(this.value); return; }
		if (matchesKey(data, "backspace")) { this.value = this.value.slice(0, -1); return; }
		if (typeof data === "string" && data.length > 0 && !/[\\x00-\\x1f]/.test(data)) this.value += data;
	}
	render(width) {
		const line = "> " + this.value;
		return [line.length > width ? line.slice(0, width) : line];
	}
}
class SelectList {
	constructor(items, maxVisible) {
		this.items = Array.isArray(items) ? items.slice() : [];
		this.maxVisible = maxVisible || 8;
		this.selected = 0;
		this.filter = "";
		this.onSelect = undefined;
		this.onCancel = undefined;
		this.onSelectionChange = undefined;
	}
	setFilter(filter) { this.filter = String(filter || ""); this.selected = 0; }
	setSelectedIndex(index) { this.selected = Math.max(0, Math.min(this.filtered().length - 1, index)); }
	getSelectedItem() { return this.filtered()[this.selected] || null; }
	filtered() {
		const q = this.filter.toLowerCase();
		return this.items.filter((item) => {
			const label = String(item && item.label != null ? item.label : item && item.value != null ? item.value : item || "");
			return !q || label.toLowerCase().includes(q);
		});
	}
	invalidate() {}
	handleInput(data) {
		const items = this.filtered();
		if (matchesKey(data, "up")) {
			this.selected = items.length ? (this.selected + items.length - 1) % items.length : 0;
			if (typeof this.onSelectionChange === "function") this.onSelectionChange(this.getSelectedItem());
			return;
		}
		if (matchesKey(data, "down")) {
			this.selected = items.length ? (this.selected + 1) % items.length : 0;
			if (typeof this.onSelectionChange === "function") this.onSelectionChange(this.getSelectedItem());
			return;
		}
		if (matchesKey(data, "enter")) {
			const item = this.getSelectedItem();
			if (item && typeof this.onSelect === "function") this.onSelect(item);
			return;
		}
		if (matchesKey(data, "escape")) {
			if (typeof this.onCancel === "function") this.onCancel();
		}
	}
	render(width) {
		const items = this.filtered();
		const start = Math.max(0, this.selected - this.maxVisible + 1);
		const visible = items.slice(start, start + this.maxVisible);
		if (visible.length === 0) return ["  No options"];
		return visible.map((item, index) => {
			const actual = start + index;
			const prefix = actual === this.selected ? "> " : "  ";
			const label = item && item.label != null ? item.label : item && item.value != null ? item.value : String(item);
			const desc = item && item.description ? "  " + item.description : "";
			return truncateToWidth(prefix + label + desc, width, "");
		});
	}
}
class Editor {
	constructor() {
		this.buffer = "";
		this.cursor = 0;
		this.onSubmit = undefined;
		this.onChange = undefined;
	}
	getText() { return this.buffer; }
	setText(text) {
		this.buffer = text == null ? "" : String(text);
		this.cursor = this.buffer.length;
	}
	handleInput(data) {
		if (data === "\\x7f" || data === "\\x08") {
			this.buffer = this.buffer.slice(0, -1);
			this.cursor = this.buffer.length;
			return;
		}
		if (data === "\\x1b[D") {
			this.cursor = Math.max(0, this.cursor - 1);
			return;
		}
		if (data === "\\x1b[C") {
			this.cursor = Math.min(this.buffer.length, this.cursor + 1);
			return;
		}
		if (data === "\\r" || data === "\\n") {
			if (typeof this.onSubmit === "function") this.onSubmit(this.buffer);
			return;
		}
		if (typeof data === "string" && data.length > 0 && !/[\\x00-\\x1f]/.test(data)) {
			this.buffer = this.buffer.slice(0, this.cursor) + data + this.buffer.slice(this.cursor);
			this.cursor += data.length;
		}
	}
	render(width) {
		const line = "> " + this.buffer;
		return [line.length > width ? line.slice(0, width) : line];
	}
	isShowingAutocomplete() { return false; }
}
class TUI {
	constructor() {
		this.children = [];
		this.mode = "normal";
		this._wantsTick = false;
		this.terminal = { setTitle() {} };
	}
	addChild(component) { this.children.push(component); }
	removeChild(component) { this.children = this.children.filter((item) => item !== component); }
	clear() { this.children = []; }
	setFocus() {}
	showOverlay(component, options) {
		this._overlay = { component, options: options || {} };
		const self = this;
		return {
			hide() { self._overlay = null; },
			setHidden() {},
			focus() {},
			unfocus() {},
			isFocused() { return true; },
			component,
			options: options || {},
		};
	}
	hideOverlay() {}
	requestRender() { this._wantsTick = true; }
	addInputListener() { return () => {}; }
	invalidate() {}
	render(width) {
		const lines = [];
		for (const child of this.children) {
			if (child && typeof child.render === "function") lines.push(...child.render(width));
		}
		return lines;
	}
}
function matchesKey(data, name) {
	const key = String(name || "").toLowerCase();
	if (key === "escape" || key === "esc") return data === "\\x1b";
	if (key === "enter" || key === "return") return data === "\\r" || data === "\\n";
	if (key === "backspace") return data === "\\x7f" || data === "\\x08";
	if (key === "left") return data === "\\x1b[D";
	if (key === "right") return data === "\\x1b[C";
	if (key === "up") return data === "\\x1b[A";
	if (key === "down") return data === "\\x1b[B";
	if (key === "delete") return data === "\\x1b[3~";
	return data === name;
}
function visibleWidth(text) {
	return String(text || "").replace(/\\x1b\\[[0-9;]*m/g, "").length;
}
function truncateToWidth(text, width, suffix) {
	const value = String(text || "");
	if (visibleWidth(value) <= width) return value;
	return value.slice(0, Math.max(0, width - (suffix || "").length)) + (suffix || "");
}
const exported = {
	__esModule: true,
	__piVirtual: "@earendil-works/pi-tui",
	name: "@earendil-works/pi-tui",
	version: "0.84.4",
	Container,
	Text,
	Input,
	SelectList,
	Editor,
	TUI,
	matchesKey,
	visibleWidth,
	truncateToWidth,
};
exported.default = exported;
module.exports = exported;
`;

const CODING_VIRTUAL = `
const tui = require("@earendil-works/pi-tui");
class CustomEditor extends tui.Editor {
	constructor(tuiInst, theme, keybindings) {
		super(tuiInst, theme);
		this.keybindings = keybindings || { matches() { return false; } };
		this.actionHandlers = new Map();
		this.onEscape = undefined;
		this.onCtrlD = undefined;
		this.onPasteImage = undefined;
		this.onExtensionShortcut = undefined;
	}
	onAction(action, handler) { this.actionHandlers.set(action, handler); }
	handleInput(data) {
		if (typeof this.onExtensionShortcut === "function" && this.onExtensionShortcut(data)) return;
		if (typeof this.onEscape === "function" && data === "\\x1b") { this.onEscape(); return; }
		super.handleInput(data);
	}
}
const exported = {
	__esModule: true,
	__piVirtual: "@earendil-works/pi-coding-agent",
	name: "@earendil-works/pi-coding-agent",
	version: "0.84.4",
	CustomEditor,
	Editor: tui.Editor,
};
exported.default = exported;
module.exports = exported;
`;

for (const name of VIRTUAL_SPECIFIERS) {
	const file = path.join(virtualRoot, `${name.replace(/[@/]/g, "_")}.js`);
	let source;
	if (name === "@earendil-works/pi-tui" || name === "@mariozechner/pi-tui") {
		source = TUI_VIRTUAL;
	} else if (name === "@earendil-works/pi-coding-agent" || name === "@mariozechner/pi-coding-agent") {
		source = CODING_VIRTUAL;
	} else {
		source = [
			"const exported = { __esModule: true, __piVirtual: " + JSON.stringify(name) + ", name: " + JSON.stringify(name) + ', version: "0.84.4" };',
			"exported.default = exported;",
			"module.exports = exported;",
		].join("\n");
	}
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
	const persistentMode = process.argv.includes("--persistent");
	let op = persistentMode ? "load" : (process.argv[3] || "load");
	let payload = {};
	if (!persistentMode) {
		let input = "";
		if (!process.stdin.isTTY) {
			input = fs.readFileSync(0, "utf8");
		}
		payload = input.trim() ? JSON.parse(input) : {};
	}
	const mod = await loadModule(extPath);
	const factory = typeof mod === "function" ? mod : mod.default || mod.factory;
	const recorded = {
		handlers: {},
		tools: [],
		toolHandlers: {},
		commands: [],
		commandHandlers: {},
		flags: [],
		shortcuts: [],
		shortcutHandlers: {},
		messageRenderers: {},
		markdownTransformers: [],
		entryRenderers: {},
		uiCalls: [],
		providers: [],
	};
	let editorFactory;
	let customComponent;
	let pendingCustom;
	let settledCustom;
	let lastCustomOverlay = false;
	let lastCustomOverlayOptions;
	function uiReply(kind) {
		const raw = process.env.PI_EXTENSION_UI_REPLY;
		if (raw === undefined || raw === "") return kind === "confirm" ? false : undefined;
		if (kind === "confirm") return raw === "1" || raw === "true" || raw === "yes";
		try {
			return JSON.parse(raw);
		} catch (_error) {
			return raw;
		}
	}
	function matchesKeySafe(data, name) {
		const key = String(name || "").toLowerCase();
		if (key === "escape" || key === "esc") return data === "\x1b";
		if (key === "enter" || key === "return") return data === "\r" || data === "\n";
		if (key === "backspace") return data === "\x7f" || data === "\x08";
		if (key === "left") return data === "\x1b[D";
		if (key === "right") return data === "\x1b[C";
		if (key === "up") return data === "\x1b[A";
		if (key === "down") return data === "\x1b[B";
		return data === name;
	}
	function makeUi() {
		return {
			select(title, options) {
				recorded.uiCalls.push({ op: "select", title, options });
				return Promise.resolve(uiReply("select"));
			},
			confirm(title, message) {
				recorded.uiCalls.push({ op: "confirm", title, message });
				return Promise.resolve(uiReply("confirm"));
			},
			input(title, placeholder) {
				recorded.uiCalls.push({ op: "input", title, placeholder });
				return Promise.resolve(uiReply("input"));
			},
			editor(title, prefill) {
				recorded.uiCalls.push({ op: "editor", title, prefill });
				return Promise.resolve(uiReply("editor"));
			},
			notify(message, type) {
				recorded.uiCalls.push({ op: "notify", message, type: type || "info" });
			},
			setStatus(key, text) {
				recorded.uiCalls.push({ op: "setStatus", key, text });
			},
			setWidget(key, content, options) {
				const lines = Array.isArray(content) ? content : content == null ? undefined : [String(content)];
				recorded.uiCalls.push({
					op: "setWidget",
					key,
					lines,
					placement: options && options.placement ? options.placement : "aboveEditor",
				});
			},
			setHeader(factory) {
				const lines = typeof factory === "function" ? undefined : factory;
				recorded.uiCalls.push({ op: "setHeader", lines });
			},
			setFooter(factory) {
				const lines = typeof factory === "function" ? undefined : factory;
				recorded.uiCalls.push({ op: "setFooter", lines });
			},
			setTitle(title) {
				recorded.uiCalls.push({ op: "setTitle", title });
			},
			setWorkingMessage(message) {
				recorded.uiCalls.push({ op: "setWorkingMessage", message });
			},
			setWorkingVisible() {},
			setWorkingIndicator() {},
			setHiddenThinkingLabel(label) {
				recorded.uiCalls.push({ op: "setHiddenThinkingLabel", label });
			},
			setEditorText(text) {
				recorded.uiCalls.push({ op: "setEditorText", text });
			},
			getEditorText() {
				return "";
			},
			pasteToEditor(text) {
				recorded.uiCalls.push({ op: "pasteToEditor", text });
			},
			setEditorComponent(factory) {
				editorFactory = factory;
				recorded.uiCalls.push({ op: "setEditorComponent", enabled: factory != null });
			},
			getEditorComponent() {
				return editorFactory;
			},
			addAutocompleteProvider() {},
			onTerminalInput() {
				return () => {};
			},
			async custom(factory, options) {
				const overlay = Boolean(options && options.overlay);
				const overlayOptions = options && options.overlayOptions ? options.overlayOptions : undefined;
				recorded.uiCalls.push({ op: "custom", overlay, overlayOptions });
				if (typeof factory !== "function") {
					return uiReply("custom");
				}
				const done = (value) => {
					settledCustom = value;
				};
				const tui = new (require("@earendil-works/pi-tui").TUI)();
				const component = await factory(tui, ui.theme, { matches: matchesKeySafe }, done);
				customComponent = component;
				lastCustomOverlay = overlay || Boolean(tui._overlay);
				lastCustomOverlayOptions = overlayOptions || (tui._overlay && tui._overlay.options) || undefined;
				if (!payload.snapshot || !persistentMode) {
					restoreEditor(component, payload.snapshot);
				}
				if (op === "customTick" && component && typeof component.tick === "function") {
					component.tick();
				} else if ((op === "customInput" || payload.data) && component && typeof component.handleInput === "function") {
					component.handleInput(payload.data || "");
				}
				if (settledCustom !== undefined) return settledCustom;
				pendingCustom = {
					snapshot: snapshotEditor(component),
					lines:
						component && typeof component.render === "function"
							? component.render(payload.width || 80)
							: [],
					wantsTick: Boolean(tui._wantsTick),
					overlay: lastCustomOverlay,
					overlayOptions: lastCustomOverlayOptions,
				};
				const pending = new Error("pending custom UI");
				pending.__piPendingCustom = true;
				throw pending;
			},
			theme: { fg(_role, text) { return text; }, bg(_role, text) { return text; }, bold(text) { return text; } },
			getAllThemes() {
				return [];
			},
			getTheme() {
				return undefined;
			},
			setTheme() {
				return { success: true };
			},
			getToolsExpanded() {
				return false;
			},
			setToolsExpanded() {},
		};
	}
	const ui = makeUi();
	const pi = {
		on(event, handler) {
			if (!recorded.handlers[event]) recorded.handlers[event] = [];
			recorded.handlers[event].push(handler);
		},
		registerTool(tool) {
			recorded.tools.push({
				name: tool.name,
				description: tool.description || "",
				parameters: tool.parameters || null,
			});
			if (typeof tool.execute === "function") {
				recorded.toolHandlers[tool.name] = tool.execute;
			} else if (typeof tool.handler === "function") {
				recorded.toolHandlers[tool.name] = tool.handler;
			}
		},
		registerCommand(name, options) {
			recorded.commands.push({ name, description: (options && options.description) || "" });
			if (options && typeof options.handler === "function") {
				recorded.commandHandlers[name] = options.handler;
			}
		},
		registerProvider(nameOrProvider, config) {
			if (typeof nameOrProvider === "string") {
				recorded.providers.push({ name: nameOrProvider, config: config || {} });
			} else if (nameOrProvider && typeof nameOrProvider === "object") {
				recorded.providers.push({
					name: nameOrProvider.id || nameOrProvider.name || "custom",
					config: nameOrProvider,
				});
			}
		},
		ui,
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
		unregisterProvider(name) {
			recorded.providers = recorded.providers.filter((provider) => provider.name !== name);
		},
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
	function snapshotEditor(editor) {
		const extra = {};
		for (const key of Object.keys(editor || {})) {
			const value = editor[key];
			if (typeof value === "function" || key === "actionHandlers" || key === "keybindings") continue;
			if (typeof value === "string" || typeof value === "number" || typeof value === "boolean" || value == null) {
				extra[key] = value;
			}
		}
		return {
			text: editor && typeof editor.getText === "function" ? editor.getText() : (editor && editor.buffer) || "",
			extra,
		};
	}
	function restoreEditor(editor, snap) {
		if (!editor || !snap) return;
		if (typeof editor.setText === "function") editor.setText(snap.text || "");
		if (snap.extra) Object.assign(editor, snap.extra);
	}
	async function activateEditor() {
		const ctx = { ui, mode: "tui" };
		for (const handler of recorded.handlers.session_start || []) {
			await handler({}, ctx);
		}
		if (typeof editorFactory !== "function") return undefined;
		const editor = editorFactory({}, ui.theme, {
			matches() {
				return false;
			},
		});
		restoreEditor(editor, payload.snapshot);
		return editor;
	}
	async function runOps() {
	if ((op === "customTick" || op === "customInput") && customComponent && persistentMode) {
		if (settledCustom !== undefined) {
			result = settledCustom;
			return;
		}
		if (op === "customTick" && typeof customComponent.tick === "function") {
			customComponent.tick();
		} else if (typeof customComponent.handleInput === "function") {
			customComponent.handleInput(payload.data || "");
		}
		if (settledCustom !== undefined) {
			result = settledCustom;
			return;
		}
		pendingCustom = {
			snapshot: snapshotEditor(customComponent),
			lines:
				customComponent && typeof customComponent.render === "function"
					? customComponent.render(payload.width || 80)
					: [],
			wantsTick: true,
			overlay: lastCustomOverlay,
			overlayOptions: lastCustomOverlayOptions,
		};
		result = Object.assign({ pending: true }, pendingCustom);
		return;
	}
	if (op === "emit") {
		const eventName = payload.type || payload.event;
		const ctx = Object.assign({ ui, mode: "tui" }, payload.ctx || {});
		ctx.ui = ui;
		for (const handler of recorded.handlers[eventName] || []) {
			result = await handler(payload, ctx);
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
			const ctx = payload.ctx || {};
			ctx.ui = ctx.ui || ui;
			result = await handler(ctx);
		}
	} else if (op === "command" || op === "customInput" || op === "customTick") {
		const name = String(payload.name || "");
		const handler = recorded.commandHandlers[name];
		if (typeof handler === "function") {
			const ctx = payload.ctx || { mode: "tui" };
			ctx.ui = ctx.ui || ui;
			try {
				result = await handler(payload.args || "", ctx);
			} catch (error) {
				if (error && error.__piPendingCustom && pendingCustom) {
					result = Object.assign({ pending: true }, pendingCustom);
				} else {
					throw error;
				}
			}
		}
	} else if (op === "tool") {
		const handler = recorded.toolHandlers[payload.name];
		if (typeof handler === "function") {
			const out = await handler(
				payload.toolCallId || "call",
				payload.args || {},
				undefined,
				undefined,
				payload.ctx || { cwd: payload.cwd },
			);
			if (typeof out === "string") {
				result = { content: out, isError: false };
			} else if (out && Array.isArray(out.content)) {
				result = {
					content: out.content.map((block) => block.text || "").join("\n"),
					isError: Boolean(out.isError),
					details: out.details,
				};
			} else if (out && typeof out.content === "string") {
				result = {
					content: out.content,
					isError: Boolean(out.isError),
					details: out.details,
				};
			} else {
				result = out;
			}
		}
	} else if (op === "editorInput" || op === "editorRender") {
		const editor = await activateEditor();
		if (!editor) {
			result = { enabled: false };
		} else {
			let submitted;
			let aborted;
			if (typeof editor === "object") {
				editor.onSubmit = (text) => {
					submitted = text == null ? "" : String(text);
				};
				if (typeof editor.onEscape !== "function") {
					editor.onEscape = () => {
						aborted = true;
					};
				}
			}
			if (op === "editorInput") {
				editor.handleInput(payload.data || "");
			}
			const snap = snapshotEditor(editor);
			let action;
			if (submitted !== undefined) action = "submit";
			else if (aborted) action = "abort";
			result = {
				enabled: true,
				text: submitted !== undefined ? submitted : snap.text,
				snapshot: snap,
				lines:
					typeof editor.render === "function"
						? editor.render(payload.width || 80)
						: ["> " + snap.text],
				action,
			};
		}
	}
	}
	function emitResult() {
		return {
			ok: true,
			handlers: Object.keys(recorded.handlers),
			tools: recorded.tools,
			commands: recorded.commands,
			flags: recorded.flags,
			shortcuts: recorded.shortcuts,
			messageRenderers: Object.keys(recorded.messageRenderers),
			entryRenderers: Object.keys(recorded.entryRenderers),
			markdownTransformers: recorded.markdownTransformers.length,
			uiCalls: recorded.uiCalls,
			hasEditor: typeof editorFactory === "function",
			hasCustom: Boolean(customComponent),
			providers: recorded.providers,
			result,
		};
	}
	await runOps();
	if (persistentMode) {
		process.stdout.write(JSON.stringify(emitResult()) + "\n");
		const readline = require("readline");
		const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
		for await (const line of rl) {
			if (!line.trim()) continue;
			const msg = JSON.parse(line);
			op = msg.op;
			payload = msg.payload || {};
			recorded.uiCalls = [];
			result = null;
			pendingCustom = undefined;
			await runOps();
			process.stdout.write(JSON.stringify(emitResult()) + "\n");
		}
	} else {
		process.stdout.write(JSON.stringify(emitResult()));
	}
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
