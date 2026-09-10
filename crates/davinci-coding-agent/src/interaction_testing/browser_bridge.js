// Bounded Playwright browser bridge adapter executing over stdio JSONL.
// Restricts all traffic to the authorized fixture origin.
// Captures console errors, network failures, DOM snapshots, and screenshots.

const readline = require('readline');

async function main() {
    let playwright;
    try {
        playwright = require('playwright');
    } catch (err) {
        process.stdout.write(JSON.stringify({ type: 'error', message: 'Playwright runtime not installed' }) + '\n');
        process.exit(1);
    }

    const rl = readline.createInterface({
        input: process.stdin,
        output: process.stdout,
        terminal: false
    });

    let browser = null;
    let context = null;
    let page = null;
    let fixtureOrigin = null;

    for await (const line of rl) {
        if (!line.trim()) continue;
        let command;
        try {
            command = JSON.parse(line);
        } catch (e) {
            process.stdout.write(JSON.stringify({ type: 'error', message: 'Malformed JSON command' }) + '\n');
            continue;
        }

        try {
            switch (command.action) {
                case 'launch': {
                    fixtureOrigin = command.fixture_origin;
                    browser = await playwright.chromium.launch({ headless: true });
                    context = await browser.newContext({ ignoreHTTPSErrors: true });
                    page = await context.newPage();

                    // Security: Route interception enforcing fixture origin restriction
                    await page.route('**/*', (route) => {
                        const reqUrl = route.request().url();
                        try {
                            const parsed = new URL(reqUrl);
                            if (fixtureOrigin && parsed.origin !== fixtureOrigin) {
                                route.abort('accessdenied');
                                process.stdout.write(JSON.stringify({
                                    type: 'network_failure',
                                    url: reqUrl,
                                    status: 403,
                                    failure_text: 'Blocked by origin policy: ' + reqUrl
                                }) + '\n');
                                return;
                            }
                        } catch {
                            route.abort('blockedbyclient');
                            return;
                        }
                        route.continue();
                    });

                    page.on('console', (msg) => {
                        if (msg.type() === 'error') {
                            process.stdout.write(JSON.stringify({ type: 'console_error', message: msg.text() }) + '\n');
                        }
                    });

                    page.on('requestfailed', (req) => {
                        process.stdout.write(JSON.stringify({
                            type: 'network_failure',
                            url: req.url(),
                            status: 0,
                            failure_text: req.failure() ? req.failure().errorText : 'request failed'
                        }) + '\n');
                    });

                    process.stdout.write(JSON.stringify({ type: 'ready' }) + '\n');
                    break;
                }
                case 'navigate': {
                    const resp = await page.goto(command.url, { waitUntil: 'load', timeout: 5000 });
                    process.stdout.write(JSON.stringify({
                        type: 'navigated',
                        url: command.url,
                        status: resp ? resp.status() : 200
                    }) + '\n');
                    break;
                }
                case 'click': {
                    await page.locator(command.selector).click({ timeout: 5000 });
                    process.stdout.write(JSON.stringify({ type: 'action_done' }) + '\n');
                    break;
                }
                case 'type': {
                    await page.locator(command.selector).fill(command.text, { timeout: 5000 });
                    process.stdout.write(JSON.stringify({ type: 'action_done' }) + '\n');
                    break;
                }
                case 'assert_dom': {
                    const actual = await page.locator(command.selector).innerText({ timeout: 5000 });
                    const passed = actual.includes(command.expected);
                    process.stdout.write(JSON.stringify({
                        type: 'assertion_result',
                        selector: command.selector,
                        passed,
                        actual
                    }) + '\n');
                    break;
                }
                case 'capture_snapshot': {
                    const html = await page.content();
                    process.stdout.write(JSON.stringify({ type: 'dom_snapshot', html }) + '\n');
                    break;
                }
                case 'capture_screenshot': {
                    const buffer = await page.screenshot({ type: 'png' });
                    process.stdout.write(JSON.stringify({ type: 'screenshot', base64: buffer.toString('base64') }) + '\n');
                    break;
                }
                case 'close': {
                    if (browser) await browser.close();
                    process.stdout.write(JSON.stringify({ type: 'closed' }) + '\n');
                    process.exit(0);
                    break;
                }
                default: {
                    process.stdout.write(JSON.stringify({ type: 'error', message: 'Unknown action: ' + command.action }) + '\n');
                }
            }
        } catch (err) {
            process.stdout.write(JSON.stringify({ type: 'error', message: err.message }) + '\n');
        }
    }
}

if (require.main === module) {
    main().catch(err => {
        process.stderr.write('Fatal bridge error: ' + err.stack + '\n');
        process.exit(1);
    });
}
