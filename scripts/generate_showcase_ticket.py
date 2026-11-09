import os
import time
import base64
from playwright.sync_api import sync_playwright

os.environ["no_proxy"] = "localhost,127.0.0.1"
os.environ["NO_PROXY"] = "localhost,127.0.0.1"

def generate_ticket():
    print("[1] Launching browser...")
    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=True,
            args=["--no-sandbox", "--disable-setuid-sandbox"]
        )
        context = browser.new_context(
            viewport={"width": 1400, "height": 900},
            device_scale_factor=2
        )
        page = context.new_page()

        print("[2] Navigating to http://127.0.0.1:8000 ...")
        page.goto("http://127.0.0.1:8000")
        page.wait_for_selector("#app")
        page.wait_for_timeout(1000)

        # Switch to Portuguese (or English)
        pt_btn = page.get_by_role("button", name="PT", exact=True)
        if pt_btn.is_visible():
            pt_btn.click()
            page.wait_for_timeout(300)

        # Add Interstellar
        print("[3] Adding Interstellar seed...")
        search_input = page.locator("input[type='text']")
        search_input.click()
        search_input.fill("Interstellar")
        search_input.press("Enter")
        page.wait_for_timeout(500)

        first_row = page.locator(".absolute .group").first
        first_row.wait_for(state="visible", timeout=5000)
        first_row.click()
        page.wait_for_timeout(1500)

        # Click on Passaporte Cultural button
        print("[4] Opening Cultural Passport modal...")
        ticket_btn = page.locator("button:has-text('Passaporte Cultural')")
        ticket_btn.wait_for(state="visible", timeout=5000)
        ticket_btn.click()
        page.wait_for_timeout(1500)

        # Extract constellation canvas
        constellation_url = page.evaluate("() => document.querySelectorAll('canvas')[0].toDataURL('image/png')")
        c_header, c_encoded = constellation_url.split(",", 1)
        with open("docs/showcase_constellation.png", "wb") as f:
            f.write(base64.b64decode(c_encoded))
        print("[5] Saved constellation PNG to docs/showcase_constellation.png")

        # Extract ticket receipt canvas specifically
        ticket_url = page.evaluate("() => { const canvases = document.querySelectorAll('canvas'); return (canvases.length > 1 ? canvases[1] : canvases[0]).toDataURL('image/png'); }")
        t_header, t_encoded = ticket_url.split(",", 1)
        with open("docs/showcase_ticket.png", "wb") as f:
            f.write(base64.b64decode(t_encoded))
        print("[6] Saved pure high-res ticket PNG to docs/showcase_ticket.png")

        browser.close()

if __name__ == "__main__":
    generate_ticket()
