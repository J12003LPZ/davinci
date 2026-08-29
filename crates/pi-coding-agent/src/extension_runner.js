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
function serializeOverlayOptions(options, width, height) {
	if (!options || typeof options !== "object") return options;
	const out = Object.assign({}, options);
	if (typeof options.visible === "function") {
		try {
			out.visible = Boolean(options.visible(width || 80, height || 24));
		} catch (_error) {
			out.visible = true;
		}
	}
	return out;
}
class TUI {
	constructor() {
		this.children = [];
		this.mode = "normal";
		this._wantsTick = false;
		this.terminal = { setTitle() {}, columns: 80, rows: 24 };
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

function serializeOverlayOptions(options, width, height) {
	if (!options || typeof options !== "object") return options;
	const out = Object.assign({}, options);
	if (typeof options.visible === "function") {
		try {
			out.visible = Boolean(options.visible(width || 80, height || 24));
		} catch (_error) {
			out.visible = true;
		}
	}
	return out;
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
		flagDefaults: {},
		shortcuts: [],
		shortcutHandlers: {},
		messageRenderers: {},
		markdownTransformers: [],
		entryRenderers: {},
		uiCalls: [],
		sessionCalls: [],
		providers: [],
		autocompleteProviders: [],
		eventEmits: [],
		unregisteredProviders: [],
		terminalInputHandlers: [],
		currentTheme: payload.theme,
		toolsExpanded: Boolean(payload.toolsExpanded),
		streamHandlers: {},
		refreshHandlers: {},
		oauthLogin: {},
		oauthRefresh: {},
		oauthGetApiKey: {},
		editorText: typeof payload.editorText === "string" ? payload.editorText : "",
		toolRenderers: {},
		toolUpdates: [],
	};
	const eventBus = {};
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
				const lines =
					content == null
						? undefined
						: typeof content === "function" || Array.isArray(content)
							? renderFactory(content, [], options && options.width)
							: [String(content)];
				recorded.uiCalls.push({
					op: "setWidget",
					key,
					lines,
					factory: typeof content === "function",
					placement: options && options.placement ? options.placement : "aboveEditor",
				});
			},
			setHeader(factory) {
				const lines = factory == null ? undefined : renderFactory(factory, []);
				recorded.uiCalls.push({
					op: "setHeader",
					lines,
					factory: typeof factory === "function",
				});
			},
			setFooter(factory) {
				const lines = factory == null ? undefined : renderFactory(factory, [{}]);
				recorded.uiCalls.push({
					op: "setFooter",
					lines,
					factory: typeof factory === "function",
				});
			},
			setTitle(title) {
				recorded.uiCalls.push({ op: "setTitle", title });
			},
			setWorkingMessage(message) {
				recorded.uiCalls.push({ op: "setWorkingMessage", message });
			},
			setWorkingVisible(visible) {
				recorded.uiCalls.push({ op: "setWorkingVisible", visible: Boolean(visible) });
			},
			setWorkingIndicator(options) {
				recorded.uiCalls.push({
					op: "setWorkingIndicator",
					options: options === undefined ? null : options,
				});
			},
			setHiddenThinkingLabel(label) {
				recorded.uiCalls.push({ op: "setHiddenThinkingLabel", label });
			},
			setEditorText(text) {
				recorded.uiCalls.push({ op: "setEditorText", text });
			},
			getEditorText() {
				if (typeof recorded.editorText === "string") {
					return recorded.editorText;
				}
				if (typeof process.env.PI_EXTENSION_EDITOR_TEXT === "string") {
					return process.env.PI_EXTENSION_EDITOR_TEXT;
				}
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
			addAutocompleteProvider(factory) {
				try {
					const current = {
						triggerCharacters: ["@"],
						getSuggestions: async () => null,
						applyCompletion() {
							return { lines: [], cursorLine: 0, cursorCol: 0 };
						},
					};
					const wrapped = typeof factory === "function" ? factory(current) : factory;
					if (!wrapped) return;
					const triggers =
						Array.isArray(wrapped.triggerCharacters) && wrapped.triggerCharacters.length
							? wrapped.triggerCharacters
							: ["#"];
					let items = [];
					const fixture = process.env.PI_EXTENSION_AUTOCOMPLETE_REPLY;
					if (fixture) {
						try {
							items = JSON.parse(fixture);
						} catch {}
					} else if (typeof wrapped.getSuggestions === "function") {
						try {
							const result = wrapped.getSuggestions([""], 0, 0, { signal: { aborted: false } });
							if (result && typeof result.then !== "function" && Array.isArray(result.items)) {
								items = result.items;
							}
						} catch {}
					}
					recorded.autocompleteProviders.push({
						triggerCharacters: triggers,
						items,
						getSuggestions:
							typeof wrapped.getSuggestions === "function" ? wrapped.getSuggestions : null,
					});
				} catch {}
			},
			onTerminalInput(handler) {
				if (typeof handler === "function") {
					recorded.terminalInputHandlers.push(handler);
				}
				recorded.uiCalls.push({ op: "onTerminalInput" });
				return () => {
					recorded.terminalInputHandlers = recorded.terminalInputHandlers.filter(
						(item) => item !== handler,
					);
				};
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
				tui.terminal.columns = payload.width || 80;
				tui.terminal.rows = payload.height || 24;
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
					overlayOptions: serializeOverlayOptions(
						lastCustomOverlayOptions,
						payload.width,
						payload.height
					),
				};
				const pending = new Error("pending custom UI");
				pending.__piPendingCustom = true;
				throw pending;
			},
			theme: { fg(_role, text) { return text; }, bg(_role, text) { return text; }, bold(text) { return text; } },
			getAllThemes() {
				return Array.isArray(payload.themes) ? payload.themes : [];
			},
			getTheme() {
				if (recorded.currentTheme !== undefined) return recorded.currentTheme;
				return payload.theme;
			},
			setTheme(theme) {
				if (theme && typeof theme === "object") {
					recorded.currentTheme = theme.name || "<in-memory>";
					recorded.uiCalls.push({ op: "setTheme", theme, success: true });
					return { success: true };
				}
				const name = theme == null ? "" : String(theme);
				const themes = Array.isArray(payload.themes) ? payload.themes : [];
				const found = themes.some((item) => item && item.name === name);
				if (name && themes.length > 0 && !found) {
					recorded.uiCalls.push({
						op: "setTheme",
						name,
						success: false,
						error: "Theme not found: " + name,
					});
					return { success: false, error: "Theme not found: " + name };
				}
				recorded.currentTheme = name;
				recorded.uiCalls.push({ op: "setTheme", name, success: true });
				return { success: true };
			},
			getToolsExpanded() {
				return Boolean(recorded.toolsExpanded);
			},
			setToolsExpanded(expanded) {
				recorded.toolsExpanded = Boolean(expanded);
				recorded.uiCalls.push({ op: "setToolsExpanded", expanded: Boolean(expanded) });
			},
		};
	}
	const ui = makeUi();
	function renderFactory(factory, extraArgs, width) {
		if (factory == null) return undefined;
		if (Array.isArray(factory)) return factory.map((line) => String(line));
		if (typeof factory !== "function") return [String(factory)];
		try {
			const { TUI } = require("@earendil-works/pi-tui");
			const tui = new TUI();
			tui.terminal = { columns: width || payload.width || 80, rows: payload.height || 24 };
			const component = factory(tui, {}, ...(extraArgs || []));
			if (!component) return [];
			if (typeof component.render === "function") return component.render(width || payload.width || 80);
			if (Array.isArray(component)) return component.map((line) => String(line));
			return [String(component)];
		} catch {
			return [];
		}
	}
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
				executionMode: tool.executionMode || null,
				renderShell: tool.renderShell || "default",
				hasRenderCall: typeof tool.renderCall === "function",
				hasRenderResult: typeof tool.renderResult === "function",
			});
			recorded.toolRenderers[tool.name] = {
				renderCall: tool.renderCall,
				renderResult: tool.renderResult,
			};
			if (typeof tool.execute === "function") {
				recorded.toolHandlers[tool.name] = tool.execute;
			} else if (typeof tool.handler === "function") {
				recorded.toolHandlers[tool.name] = tool.handler;
			}
		},
		registerCommand(name, options) {
			const rec = {
				name,
				description: (options && options.description) || "",
				argumentItems: [],
			};
			if (options && typeof options.getArgumentCompletions === "function") {
				try {
					const items = options.getArgumentCompletions("");
					if (Array.isArray(items)) rec.argumentItems = items;
				} catch {}
			}
			recorded.commands.push(rec);
			if (options && typeof options.handler === "function") {
				recorded.commandHandlers[name] = options.handler;
			}
		},
		registerProvider(nameOrProvider, config) {
			const source =
				typeof nameOrProvider === "string" ? config || {} : nameOrProvider && typeof nameOrProvider === "object" ? nameOrProvider : {};
			const name =
				typeof nameOrProvider === "string"
					? nameOrProvider
					: (nameOrProvider && (nameOrProvider.id || nameOrProvider.name)) || "custom";
			const copy = Object.assign({}, source);
			const hasStreamSimple = typeof source.streamSimple === "function";
			const hasRefreshModels = typeof source.refreshModels === "function";
			const hasOauth = Boolean(source.oauth);
			delete copy.streamSimple;
			delete copy.refreshModels;
			if (hasStreamSimple) recorded.streamHandlers[name] = source.streamSimple;
			if (hasRefreshModels) recorded.refreshHandlers[name] = source.refreshModels;
			if (source.oauth && typeof source.oauth === "object") {
				copy.oauth = {
					name: source.oauth.name,
					id: source.oauth.id,
				};
				if (typeof source.oauth.login === "function") recorded.oauthLogin[name] = source.oauth.login;
				if (typeof source.oauth.refreshToken === "function") {
					recorded.oauthRefresh[name] = source.oauth.refreshToken;
				}
				if (typeof source.oauth.getApiKey === "function") {
					recorded.oauthGetApiKey[name] = source.oauth.getApiKey;
				}
			}
			recorded.providers.push({
				name,
				config: copy,
				hasStreamSimple,
				hasRefreshModels,
				hasOauth,
			});
		},
		ui,
		registerFlag(name, options) {
			recorded.flags.push(name);
			if (options && options.default !== undefined) {
				recorded.flagDefaults[name] = options.default;
			}
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
		sendMessage(message, options) {
			recorded.sessionCalls.push({ op: "sendMessage", message, options: options || {} });
		},
		sendUserMessage(text, options) {
			recorded.sessionCalls.push({
				op: "sendUserMessage",
				text,
				options: options || {},
			});
		},
		appendEntry(customType, data) {
			recorded.sessionCalls.push({ op: "appendEntry", customType, data: data ?? null });
		},
		setSessionName(name) {
			recorded.sessionCalls.push({ op: "setSessionName", name });
		},
		getSessionName() {
			return payload.sessionName;
		},
		setLabel(entryId, label) {
			recorded.sessionCalls.push({ op: "setLabel", entryId, label });
		},
		newSession(options) {
			recorded.sessionCalls.push({ op: "newSession", options: options || {} });
			return { cancelled: false };
		},
		fork(entryId, options) {
			recorded.sessionCalls.push({ op: "fork", entryId, options: options || {} });
			return { cancelled: false };
		},
		switchSession(sessionPath, options) {
			recorded.sessionCalls.push({
				op: "switchSession",
				sessionPath,
				options: options || {},
			});
			return Promise.resolve({ cancelled: false });
		},
		navigateTree(targetId, options) {
			recorded.sessionCalls.push({
				op: "navigateTree",
				targetId,
				options: options || {},
			});
			return Promise.resolve({ cancelled: false });
		},
		reload() {
			recorded.sessionCalls.push({ op: "reload" });
			return Promise.resolve();
		},
		waitForIdle() {
			recorded.sessionCalls.push({ op: "waitForIdle" });
			return Promise.resolve();
		},
		exec(command, args, options) {
			const cwd = (options && options.cwd) || payload.cwd || process.cwd();
			const reply = process.env.PI_EXTENSION_EXEC_REPLY;
			if (reply !== undefined) {
				recorded.sessionCalls.push({
					op: "exec",
					command,
					args: args || [],
					cwd,
					stdout: reply,
					code: 0,
				});
				return Promise.resolve({ code: 0, stdout: reply, stderr: "" });
			}
			const { spawnSync } = require("child_process");
			const argv = Array.isArray(args) && args.length ? [command, ...args] : null;
			const ran = argv
				? spawnSync(argv[0], argv.slice(1), { encoding: "utf8", cwd, shell: false })
				: spawnSync(String(command), { encoding: "utf8", cwd, shell: true });
			const stdout = ran.stdout || "";
			const stderr = ran.stderr || "";
			const code = ran.status == null ? 1 : ran.status;
			recorded.sessionCalls.push({
				op: "exec",
				command,
				args: args || [],
				cwd,
				stdout,
				code,
			});
			return Promise.resolve({ code, stdout, stderr });
		},
		getActiveTools() {
			return Array.isArray(payload.activeTools) ? payload.activeTools : [];
		},
		getAllTools() {
			if (Array.isArray(payload.allTools)) return payload.allTools;
			return Array.isArray(payload.activeTools) ? payload.activeTools : [];
		},
		setActiveTools(toolNames) {
			recorded.sessionCalls.push({
				op: "setActiveTools",
				toolNames: Array.isArray(toolNames) ? toolNames : [],
			});
		},
		getCommands() {
			return Array.isArray(payload.commands) ? payload.commands : [];
		},
		setModel(model) {
			const id =
				typeof model === "string"
					? model
					: model && (model.id || model.name || model.model);
			const provider = typeof model === "object" && model ? model.provider : undefined;
			recorded.sessionCalls.push({ op: "setModel", model: id, provider });
			return Promise.resolve(true);
		},
		getThinkingLevel() {
			return payload.thinkingLevel || "off";
		},
		setThinkingLevel(level) {
			recorded.sessionCalls.push({ op: "setThinkingLevel", level: String(level || "off") });
		},
		unregisterProvider(name) {
			recorded.providers = recorded.providers.filter((provider) => provider.name !== name);
			if (name) recorded.unregisteredProviders.push(String(name));
			recorded.sessionCalls.push({ op: "unregisterProvider", name: String(name || "") });
		},
		getFlag(name) {
			if (!recorded.flags.includes(name)) return undefined;
			const flags = payload.flagValues || {};
			if (Object.prototype.hasOwnProperty.call(flags, name)) return flags[name];
			return recorded.flagDefaults[name];
		},
		events: {
			emit(channel, data) {
				recorded.eventEmits.push({ channel, data });
				for (const handler of eventBus[channel] || []) {
					try {
						handler(data);
					} catch {}
				}
			},
			on(channel, handler) {
				if (!eventBus[channel]) eventBus[channel] = [];
				eventBus[channel].push(handler);
				return () => {
					eventBus[channel] = (eventBus[channel] || []).filter((item) => item !== handler);
				};
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
			overlayOptions: serializeOverlayOptions(
				lastCustomOverlayOptions,
				payload.width,
				payload.height
			),
		};
		result = Object.assign({ pending: true }, pendingCustom);
		return;
	}
	if (op === "emit") {
		const eventName = payload.type || payload.event;
		const ctx = Object.assign({ ui, mode: "tui" }, payload.ctx || {});
		ctx.ui = ui;
		let current = payload;
		for (const handler of recorded.handlers[eventName] || []) {
			result = await handler(current, ctx);
			if (result && (result.block || result.cancel || result.action === "handled")) {
				break;
			}
			if (result && result.action === "transform" && eventName === "input") {
				current = Object.assign({}, current, {
					text: result.text,
					images: result.images !== undefined ? result.images : current.images,
				});
				result = {
					action: "transform",
					text: current.text,
					images: current.images,
				};
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
			ctx.sendMessage = pi.sendMessage;
			ctx.sendUserMessage = pi.sendUserMessage;
			ctx.appendEntry = pi.appendEntry;
			ctx.setSessionName = pi.setSessionName;
			ctx.getSessionName = pi.getSessionName;
			ctx.setLabel = pi.setLabel;
			ctx.newSession = pi.newSession;
			ctx.fork = pi.fork;
			ctx.exec = pi.exec;
			ctx.switchSession = pi.switchSession;
			ctx.navigateTree = pi.navigateTree;
			ctx.reload = pi.reload;
			ctx.waitForIdle = pi.waitForIdle;
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
	} else if (op === "renderToolCall" || op === "renderToolResult") {
		const renderer = recorded.toolRenderers[payload.name] || {};
		const fn = op === "renderToolCall" ? renderer.renderCall : renderer.renderResult;
		if (typeof fn === "function") {
			try {
				const component =
					op === "renderToolCall"
						? fn(payload.args || {}, {}, payload.context || {})
						: fn(payload.result || {}, { expanded: Boolean(payload.expanded) }, {}, payload.context || {});
				result = {
					lines:
						component && typeof component.render === "function"
							? component.render(payload.width || 80)
							: Array.isArray(component)
								? component.map((line) => String(line))
								: component == null
									? []
									: [String(component)],
				};
			} catch (error) {
				result = { lines: [], error: error && error.message ? error.message : String(error) };
			}
		} else {
			result = { lines: [] };
		}
	} else if (op === "tool") {
		const handler = recorded.toolHandlers[payload.name];
		if (typeof handler === "function") {
			recorded.toolUpdates = [];
			const onUpdate = (partial) => {
				recorded.toolUpdates.push(partial);
			};
			const out = await handler(
				payload.toolCallId || "call",
				payload.args || {},
				undefined,
				onUpdate,
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
	} else if (op === "autocomplete") {
		const text = String(payload.text || payload.prefix || "");
		const lines = text.split("\n");
		const cursorLine = Math.max(0, lines.length - 1);
		const cursorCol = lines[cursorLine] ? lines[cursorLine].length : 0;
		const fixture = process.env.PI_EXTENSION_AUTOCOMPLETE_REPLY;
		if (fixture) {
			try {
				result = { items: JSON.parse(fixture) };
			} catch {
				result = { items: [] };
			}
		} else {
			const items = [];
			for (const provider of recorded.autocompleteProviders) {
				if (typeof provider.getSuggestions === "function") {
					const out = await provider.getSuggestions(lines, cursorLine, cursorCol, {
						signal: { aborted: false },
					});
					if (out && Array.isArray(out.items)) items.push(...out.items);
				} else if (Array.isArray(provider.items)) {
					items.push(...provider.items);
				}
			}
			result = { items };
		}
	} else if (op === "event") {
		const channel = String(payload.channel || "");
		let delivered = 0;
		for (const handler of eventBus[channel] || []) {
			try {
				handler(payload.data);
				delivered += 1;
			} catch {}
		}
		result = { delivered };
	} else if (op === "streamSimple") {
		const handler = recorded.streamHandlers[payload.provider];
		if (typeof handler === "function") {
			const out = await handler(payload.model || {}, payload.context || {}, payload.options || {});
			if (out && typeof out[Symbol.asyncIterator] === "function") {
				let text = "";
				let message;
				for await (const ev of out) {
					if (ev && ev.type === "text_delta" && ev.delta) text += ev.delta;
					if (ev && ev.type === "text" && ev.text) text += ev.text;
					if (ev && ev.message) message = ev.message;
				}
				result = message || {
					role: "assistant",
					content: [{ type: "text", text }],
				};
			} else if (out && (out.content || out.text)) {
				result = out;
			} else {
				result = {
					role: "assistant",
					content: [{ type: "text", text: out == null ? "" : String(out) }],
				};
			}
		}
	} else if (op === "refreshModels") {
		const handler = recorded.refreshHandlers[payload.provider];
		if (typeof handler === "function") {
			const context = Object.assign(
				{
					allowNetwork: true,
					force: false,
					signal: { aborted: false },
					publish: async () => true,
				},
				payload.context || {},
			);
			result = { models: await handler(context) };
		}
	} else if (op === "oauthLogin") {
		const fixture = process.env.PI_EXTENSION_OAUTH_REPLY;
		if (fixture) {
			try {
				result = JSON.parse(fixture);
			} catch {
				result = { access: fixture, refresh: "", expires: 0 };
			}
		} else {
			const handler = recorded.oauthLogin[payload.provider];
			const callbacks = {
				onAuth(info) {
					recorded.uiCalls.push({ op: "oauthOnAuth", info: info || {} });
				},
				onDeviceCode(info) {
					recorded.uiCalls.push({ op: "oauthOnDeviceCode", info: info || {} });
				},
				onPrompt() {
					return Promise.resolve(process.env.PI_OAUTH_CODE || "");
				},
				onProgress() {},
				onManualCodeInput() {
					return Promise.resolve(process.env.PI_OAUTH_CODE || "");
				},
				onSelect() {
					return Promise.resolve(process.env.PI_EXTENSION_UI_REPLY || undefined);
				},
				signal: { aborted: false },
			};
			if (typeof handler === "function") {
				result = await handler(callbacks);
			}
		}
	} else if (op === "oauthRefresh") {
		const fixture = process.env.PI_EXTENSION_OAUTH_REFRESH_REPLY;
		if (fixture) {
			try {
				result = JSON.parse(fixture);
			} catch {
				result = { access: fixture, refresh: payload.credentials && payload.credentials.refresh, expires: 0 };
			}
		} else {
			const handler = recorded.oauthRefresh[payload.provider];
			if (typeof handler === "function") {
				result = await handler(payload.credentials || {}, { aborted: false });
			}
		}
	} else if (op === "oauthGetApiKey") {
		const handler = recorded.oauthGetApiKey[payload.provider];
		if (typeof handler === "function") {
			result = { apiKey: handler(payload.credentials || {}) };
		} else if (payload.credentials && payload.credentials.access) {
			result = { apiKey: payload.credentials.access };
		}
	} else if (op === "terminalInput") {
		let consumed = false;
		const data = payload.data == null ? "" : String(payload.data);
		for (const handler of recorded.terminalInputHandlers) {
			try {
				if (handler(data) === true) {
					consumed = true;
					break;
				}
			} catch {}
		}
		result = { consumed };
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
			sessionCalls: recorded.sessionCalls,
			eventEmits: recorded.eventEmits,
			hasEditor: typeof editorFactory === "function",
			hasCustom: Boolean(customComponent),
			providers: recorded.providers,
			unregisteredProviders: recorded.unregisteredProviders,
			terminalInputHandlers: recorded.terminalInputHandlers.length,
			currentTheme: recorded.currentTheme,
			toolsExpanded: Boolean(recorded.toolsExpanded),
			autocompleteProviders: recorded.autocompleteProviders.map((provider) => ({
				triggerCharacters: provider.triggerCharacters,
				items: provider.items || [],
			})),
			updates: recorded.toolUpdates,
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
			recorded.sessionCalls = [];
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
		}) + "\n",
	);
	process.exitCode = 1;
});
