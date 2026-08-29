"use strict";

const fs = require("fs");
const path = require("path");
const { pathToFileURL } = require("url");

async function loadModule(modPath) {
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
	const recorded = { handlers: {}, tools: [], commands: [], flags: [], shortcuts: [] };
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
		registerShortcut(key) {
			recorded.shortcuts.push(key);
		},
		registerMessageRenderer() {},
		registerMarkdownTransformer() {},
		registerEntryRenderer() {},
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
	}
	process.stdout.write(
		JSON.stringify({
			ok: true,
			handlers: Object.keys(recorded.handlers),
			tools: recorded.tools,
			commands: recorded.commands,
			flags: recorded.flags,
			shortcuts: recorded.shortcuts,
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
