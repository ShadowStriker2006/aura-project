from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
HARNESS_PATH = "/src/profile-harness.html"
OVERLAY_HARNESS_PATH = "/src/overlay-harness.html"


class HarnessHandler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(PROJECT_ROOT), **kwargs)

    def do_GET(self):
        request_path = self.path.split("?", 1)[0]
        if request_path == OVERLAY_HARNESS_PATH:
            html = (PROJECT_ROOT / "src" / "overlay.html").read_text(encoding="utf-8")
            marker = '<script type="module" src="overlay.js"></script>'
            injected = (
                '<script src="/tests/tauri-profile-mock.js?v=live-overlay-3"></script>\n'
                f'  {marker}'
            )
            body = html.replace(marker, injected, 1).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return

        if request_path != HARNESS_PATH:
            return super().do_GET()

        html = (PROJECT_ROOT / "src" / "index.html").read_text(encoding="utf-8")
        marker = '<script type="module" src="main.js"></script>'
        injected = (
            '<script src="/tests/tauri-profile-mock.js?v=live-ipc-2"></script>\n'
            f'    {marker}'
        )
        body = html.replace(marker, injected, 1).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    server = ThreadingHTTPServer(("127.0.0.1", 8765), HarnessHandler)
    print("Aura dashboard harness: http://127.0.0.1:8765/src/profile-harness.html?liveTick", flush=True)
    print("Aura overlay harness: http://127.0.0.1:8765/src/overlay-harness.html?liveTick", flush=True)
    server.serve_forever()
