//! Loopback HTTP fixtures, mock HTML payloads, and DOM assertions for browser testing.

/// Standard HTML fixture with deterministic DOM structure for locator-based interaction tests.
pub fn sample_html_page() -> &'static str {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Davinci Offline Test Fixture</title>
</head>
<body>
    <header>
        <h1 id="title">Davinci Harness</h1>
        <div id="status-badge" class="badge">ready</div>
    </header>
    <main>
        <div id="content">
            <p id="greeting">Hello from offline fixture</p>
            <input id="prompt-input" type="text" placeholder="Enter prompt..." />
            <button id="submit-button">Submit</button>
            <div id="output-area" style="display: none;"></div>
        </div>
    </main>
</body>
</html>"#
}

/// HTML fixture containing client-side console error and failed fetch.
pub fn error_trigger_html_page() -> &'static str {
    r#"<!DOCTYPE html>
<html>
<head><title>Error Fixture</title></head>
<body>
    <div id="error-box">Simulating runtime error</div>
    <script>
        console.error("Test console error: failed to resolve asset");
        fetch("http://127.0.0.1:9999/non_existent_endpoint").catch(e => {
            console.error("Fetch failed: " + e.message);
        });
    </script>
</body>
</html>"#
}

/// Mock DOM snapshot captured from the fixture.
pub fn mock_dom_snapshot() -> String {
    sample_html_page().to_string()
}
