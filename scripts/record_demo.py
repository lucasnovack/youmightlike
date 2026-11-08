#!/usr/bin/env python3
import os
import glob
import subprocess
import time
from playwright.sync_api import sync_playwright

os.environ["no_proxy"] = "localhost,127.0.0.1"
os.environ["NO_PROXY"] = "localhost,127.0.0.1"

RECORDINGS_DIR = "recordings"
RAW_DIR = os.path.join(RECORDINGS_DIR, "raw")
OUTPUT_MP4 = os.path.join(RECORDINGS_DIR, "youmightlike_demo.mp4")
OUTPUT_GIF = os.path.join(RECORDINGS_DIR, "youmightlike_demo.gif")

os.makedirs(RAW_DIR, exist_ok=True)

def record():
    print("[1/3] Starting headless browser recording...")
    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=True,
            args=[
                "--no-sandbox",
                "--disable-setuid-sandbox",
                "--font-render-hinting=none"
            ]
        )
        context = browser.new_context(
            viewport={"width": 1280, "height": 720},
            record_video_dir=RAW_DIR,
            record_video_size={"width": 1280, "height": 720}
        )
        page = context.new_page()

        print("Navigating to http://127.0.0.1:8000 ...")
        page.goto("http://127.0.0.1:8000")
        page.wait_for_selector("#app")
        page.wait_for_timeout(600)

        # Switch to English immediately
        en_btn = page.get_by_role("button", name="EN", exact=True)
        if en_btn.is_visible():
            en_btn.click()
            page.wait_for_timeout(300)

        search_input = page.locator("input[type='text']")

        def add_seed_query(query: str):
            print(f"Adding seed: {query}")
            search_input.click()
            search_input.fill("")
            search_input.type(query, delay=45)
            page.wait_for_timeout(150)
            search_input.press("Enter")
            page.wait_for_timeout(300)
            # Find first result in dropdown and click
            first_row = page.locator(".absolute .group").first
            first_row.wait_for(state="visible", timeout=6000)
            page.wait_for_timeout(200)
            first_row.click()
            page.wait_for_timeout(1400)

        # 1. Add Donnie Darko (Movie)
        add_seed_query("Donnie Darko")

        # 2. Add Radiohead (Music Artist) - direct soundtrack/thematic affinity
        add_seed_query("Radiohead")

        print("Interacting with graph constellation...")
        page.wait_for_timeout(1200)

        # Smooth mouse hover on canvas
        canvas = page.locator("#constellation-canvas")
        box = canvas.bounding_box()
        if box:
            cx, cy = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
            page.mouse.move(cx, cy)
            page.wait_for_timeout(300)
            page.mouse.move(cx + 100, cy - 60, steps=10)
            page.wait_for_timeout(500)
            page.mouse.move(cx - 120, cy + 80, steps=10)
            page.wait_for_timeout(500)

        # 3. Open Cultural Taste Receipt (Receiptify modal in English)
        print("Opening English Receipt modal...")
        receipt_btn = page.locator("button:has-text('Receipt')").first
        if receipt_btn.is_visible():
            receipt_btn.click()
            page.wait_for_timeout(2600)  # Show English receipt with barcode and map
            # Close receipt
            close_btn = page.locator("button:has-text('Close'), button:has-text('Fechar')").first
            if close_btn.is_visible():
                close_btn.click()
            else:
                page.keyboard.press("Escape")
            page.wait_for_timeout(800)

        page.wait_for_timeout(500)

        context.close()
        browser.close()

    print("[2/3] Browser session finished. Finding raw video...")
    webm_files = sorted(glob.glob(os.path.join(RAW_DIR, "*.webm")), key=os.path.getmtime)
    if not webm_files:
        raise RuntimeError("No recorded webm video found in " + RAW_DIR)

    raw_video = webm_files[-1]
    print(f"Raw video saved at: {raw_video}")

    # Convert to MP4
    print(f"[3/3] Converting to high-quality MP4: {OUTPUT_MP4} ...")
    cmd_mp4 = [
        "ffmpeg", "-y",
        "-i", raw_video,
        "-c:v", "libx264",
        "-preset", "slow",
        "-crf", "20",
        "-pix_fmt", "yuv420p",
        OUTPUT_MP4
    ]
    subprocess.run(cmd_mp4, check=True)

    # Convert to optimized GIF (15 fps, 960px width, palettegen)
    print(f"Generating optimized GIF: {OUTPUT_GIF} ...")
    cmd_gif = [
        "ffmpeg", "-y",
        "-i", raw_video,
        "-vf", "fps=15,scale=960:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3",
        OUTPUT_GIF
    ]
    subprocess.run(cmd_gif, check=True)

    mp4_size = os.path.getsize(OUTPUT_MP4) / (1024 * 1024)
    gif_size = os.path.getsize(OUTPUT_GIF) / (1024 * 1024)
    print(f"\nDone! Assets created successfully:")
    print(f" - MP4 (for LinkedIn / video posts): {OUTPUT_MP4} ({mp4_size:.2f} MB)")
    print(f" - GIF (for README / GitHub / preview): {OUTPUT_GIF} ({gif_size:.2f} MB)")

if __name__ == "__main__":
    record()
