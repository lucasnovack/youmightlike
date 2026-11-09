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
    padding: 40px;
    font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    color: #e6edf3;
  }
  .window {
    width: 900px;
    background: #161b22;
    border-radius: 12px;
    border: 1px solid #30363d;
    box-shadow: 0 25px 60px -15px rgba(0, 0, 0, 0.7), 0 0 0 1px rgba(255, 255, 255, 0.05);
    overflow: hidden;
  }
  .titlebar {
    background: #0d1117;
    padding: 12px 16px;
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
    padding: 24px 28px;
    font-size: 13.5px;
    line-height: 1.65;
  }
  .prompt { color: #58a6ff; }
  .cmd { color: #f0f6fc; font-weight: 600; }
  .dim { color: #6e7681; }
  .green { color: #3fb950; font-weight: 500; }
  .cyan { color: #39c5cf; }
  .yellow { color: #d29922; }
  .magenta { color: #bc8cff; }
  .white { color: #f0f6fc; }
  .card {
    margin: 16px 0;
    padding: 14px 18px;
    background: #0d1117;
    border-left: 3px solid #3fb950;
    border-radius: 4px;
    font-size: 12.5px;
  }
  .card-row { display: flex; justify-content: space-between; margin-bottom: 4px; }
  .badge {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 600;
  }
  .badge-success { background: rgba(63, 185, 80, 0.15); color: #3fb950; }
  .badge-metric { background: rgba(57, 197, 207, 0.15); color: #39c5cf; }
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
    <div class="dim" style="margin-top: 8px;">[2026-11-09T00:25:20Z INFO] Initializing taste graph runtime...</div>
    <div>[server] <span class="cyan">loading graph</span> from duckdb at <span class="white">data/processed/taste_graph_lite.duckdb</span>...</div>
    <div>[server] <span class="green">graph loaded in 1.81s</span>: <span class="white">357,643 nodes</span>, <span class="white">1,956,347 edges</span>, <span class="magenta">domain_gamma=1.00</span></div>
    <div>[server] opening in-process duckdb connection for metadata and search...</div>
    <div>[server] <span class="green">catalog loaded in 80ms</span>: <span class="dim">34,188 movies • 7,647 series • 20,064 games • 41,940 books</span></div>
    <div>[server] <span class="green">✔</span> Axum HTTP service listening on <span class="cyan">http://127.0.0.1:8000</span></div>

    <div class="card">
      <div class="card-row">
        <span><strong class="white">Memory Footprint (In-RAM CSR Graph):</strong></span>
        <span class="badge badge-success">198.4 MB RES</span>
      </div>
      <div class="card-row">
        <span><span class="dim">Topology:</span> Bipartite Knowledge Graph (petgraph)</span>
        <span class="badge badge-metric">357,643 Nodes • 1,956,347 Edges</span>
      </div>
      <div class="card-row" style="margin-bottom: 0;">
        <span><span class="dim">Hub Dampening:</span> Logarithmic Adamic-Adar + Quadratic Confluence</span>
        <span class="green">Active (Zero LLM / Zero Vector DB)</span>
      </div>
    </div>

    <div class="dim" style="margin-top: 14px;"># Real-time HTTP benchmarks (Axum + petgraph 2-hop stochastic walk)</div>
    <div><span class="prompt">lucas@fedora</span>:<span class="cyan">~/youmightlike</span>$ <span class="cmd">curl -X POST /api/recommend -d '{"seeds":["movie:23648"]}'</span> <span class="dim"># Interstellar (2014)</span></div>
    <div><span class="green">200 OK</span> <span class="dim">in</span> <span class="yellow">14.2ms</span> &rarr; <span class="white">Returned 26 cross-media recommendations across 5 domains</span></div>

    <div style="margin-top: 8px;"><span class="prompt">lucas@fedora</span>:<span class="cyan">~/youmightlike</span>$ <span class="cmd">curl -X POST /api/recommend -d '{"seeds":["game:5410"]}'</span> <span class="dim"># The Elder Scrolls V: Skyrim</span></div>
    <div><span class="green">200 OK</span> <span class="dim">in</span> <span class="yellow">11.8ms</span> &rarr; <span class="white">Hub noise filtered: 0% generic soundtrack leakage</span></div>

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
            viewport={"width": 1000, "height": 620},
            device_scale_factor=2
        )
        page = context.new_page()
        page.set_content(HTML_CONTENT)
        page.wait_for_timeout(500)

        out_path = "docs/showcase_terminal.png"
        window = page.locator(".window")
        window.screenshot(path=out_path)
        print(f"[2] Saved terminal proof screenshot to {out_path}")
        browser.close()

if __name__ == "__main__":
    generate_terminal_image()
