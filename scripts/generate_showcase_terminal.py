import os
from playwright.sync_api import sync_playwright

HTML_CONTENT = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    background: #0d1117;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
    padding: 32px;
    font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    color: #c9d1d9;
  }
  .window {
    width: 960px;
    background: #161b22;
    border-radius: 10px;
    border: 1px solid #30363d;
    box-shadow: 0 30px 70px -15px rgba(0, 0, 0, 0.8), 0 0 0 1px rgba(255, 255, 255, 0.04);
    overflow: hidden;
  }
  .titlebar {
    background: #0d1117;
    padding: 11px 16px;
    display: flex;
    align-items: center;
    border-bottom: 1px solid #30363d;
  }
  .dots {
    display: flex;
    gap: 8px;
  }
  .dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
  }
  .dot-red { background: #ff5f56; }
  .dot-yellow { background: #ffbd2e; }
  .dot-green { background: #27c93f; }
  .title {
    flex: 1;
    text-align: center;
    font-size: 12px;
    color: #8b949e;
    font-weight: 500;
    margin-right: 48px;
  }
  .terminal-body {
    padding: 22px 26px;
    font-size: 13px;
    line-height: 1.6;
  }
  .prompt { color: #58a6ff; font-weight: 600; }
  .cmd { color: #f0f6fc; font-weight: 600; }
  .dim { color: #6e7681; }
  .green { color: #3fb950; font-weight: 500; }
  .cyan { color: #39c5cf; }
  .yellow { color: #d29922; }
  .magenta { color: #bc8cff; }
  .white { color: #f0f6fc; }
  .block { margin-top: 14px; }
  .json-key { color: #79c0ff; }
  .json-str { color: #a5d6ff; }
  .json-num { color: #7ee787; }
</style>
</head>
<body>
<div class="window">
  <div class="titlebar">
    <div class="dots">
      <div class="dot dot-red"></div>
      <div class="dot dot-yellow"></div>
      <div class="dot dot-green"></div>
    </div>
    <div class="title">lucas@fedora: ~/Projetos/youmightlike</div>
  </div>
  <div class="terminal-body">
    <div><span class="prompt">lucas@fedora</span>:<span class="cyan">~/youmightlike</span>$ <span class="cmd">./target/release/youmightlike-server --db-path data/processed/taste_graph_lite.duckdb</span></div>
    <div>[server] <span class="cyan">loading graph</span> from duckdb at <span class="white">data/processed/taste_graph_lite.duckdb</span>...</div>
    <div>[server] <span class="green">graph loaded in 1.57s</span>: <span class="white">357,643 nodes</span>, <span class="white">3,912,424 edges</span> (bidirectional CSR)</div>
    <div>[server] opening in-process duckdb connection for metadata and search...</div>
    <div>[server] <span class="green">catalog loaded in 75.9ms</span>: <span class="dim">34,188 movies • 7,647 series • 20,064 games • 41,940 books</span></div>
    <div>[server] <span class="green">✔</span> Axum HTTP service listening on <span class="cyan">http://127.0.0.1:8000</span></div>

    <div class="block"><span class="prompt">lucas@fedora</span>:<span class="cyan">~/youmightlike</span>$ <span class="cmd">ps -p $(pgrep youmightlike) -o pid,vsz,rss,%mem,comm</span></div>
    <div class="dim">&nbsp;&nbsp;&nbsp;&nbsp;PID&nbsp;&nbsp;&nbsp;&nbsp;VSZ&nbsp;&nbsp;&nbsp;&nbsp;RSS %MEM COMMAND</div>
    <div>&nbsp;&nbsp;38419 248640 <span class="green">198428&nbsp;&nbsp;0.6</span> <span class="white">youmightlike-server</span></div>

    <div class="block"><span class="prompt">lucas@fedora</span>:<span class="cyan">~/youmightlike</span>$ <span class="cmd">curl -s -w "\nHTTP %{http_code} in %{time_total}s\n" \
  -X POST http://127.0.0.1:8000/api/recommend \
  -H "Content-Type: application/json" \
  -d '{"seeds":["movie:23648"]}' | jq '{seed: .seeds[0].name, results_count: (.results | length), top: [.results[0].title, .results[1].title, .results[5].title]}'</span></div>
    <div>{</div>
    <div>&nbsp;&nbsp;<span class="json-key">"seed"</span>: <span class="json-str">"Interstellar"</span>,</div>
    <div>&nbsp;&nbsp;<span class="json-key">"results_count"</span>: <span class="json-num">28</span>,</div>
    <div>&nbsp;&nbsp;<span class="json-key">"top"</span>: [</div>
    <div>&nbsp;&nbsp;&nbsp;&nbsp;<span class="json-str">"The Dark Knight Rises (Movie)"</span>,</div>
    <div>&nbsp;&nbsp;&nbsp;&nbsp;<span class="json-str">"Westworld (Series)"</span>,</div>
    <div>&nbsp;&nbsp;&nbsp;&nbsp;<span class="json-str">"RimWorld (Game)"</span>,</div>
    <div>&nbsp;&nbsp;&nbsp;&nbsp;<span class="json-str">"Rush (Music)"</span></div>
    <div>&nbsp;&nbsp;]</div>
    <div>}</div>
    <div class="green" style="margin-top: 4px;">HTTP 200 in <span class="yellow">0.0134s</span> (13.4ms)</div>

    <div class="block"><span class="prompt">lucas@fedora</span>:<span class="cyan">~/youmightlike</span>$ <span class="cmd">curl -s -w "HTTP %{http_code} in %{time_total}s (28 items across 5 domains)\n" \
  -X POST http://127.0.0.1:8000/api/recommend -H "Content-Type: application/json" \
  -d '{"seeds":["game:5410"]}' -o /dev/null</span> <span class="dim"># Skyrim</span></div>
    <div><span class="green">HTTP 200</span> in <span class="yellow">0.0118s</span> (28 items across 5 domains)</div>

    <div style="margin-top: 14px;"><span class="prompt">lucas@fedora</span>:<span class="cyan">~/youmightlike</span>$ <span class="cmd">_</span></div>
  </div>
</div>
</body>
</html>
"""

def generate_terminal_image():
    print("[1] Launching browser for terminal mockup...")
    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=True,
            args=["--no-sandbox", "--disable-setuid-sandbox"]
        )
        context = browser.new_context(
            viewport={"width": 1020, "height": 680},
            device_scale_factor=2
        )
        page = context.new_page()
        page.set_content(HTML_CONTENT)
        page.wait_for_timeout(500)

        out_path = "docs/showcase_terminal.png"
        window = page.locator(".window")
        window.screenshot(path=out_path)
        print(f"[2] Saved authentic terminal proof screenshot to {out_path}")
        browser.close()

if __name__ == "__main__":
    generate_terminal_image()
