import hmac
import logging
import os
from pathlib import Path
import sys
from threading import Thread

root = Path(__file__).resolve().parent
sys.path[:0] = [str(root / "app"), str(root / "site")]
key = os.environ.pop("YAP_LYRICA_KEY")

from src.app import app
from werkzeug.serving import make_server


def serve_lyrics(environ, start_response):
    path = environ.get("PATH_INFO")
    supplied = environ.get("HTTP_X_YAP_LYRICA_KEY", "")
    trusted = supplied.isascii() and hmac.compare_digest(supplied, key)
    if environ["REQUEST_METHOD"] != "GET" or path not in ("/health", "/lyrics/") or not trusted:
        start_response("404 Not Found", [("Content-Length", "0")])
        return []
    return app(environ, start_response)


def watch_parent():
    sys.stdin.buffer.read()
    os._exit(0)


logging.getLogger("werkzeug").setLevel(logging.ERROR)
server = make_server("127.0.0.1", 0, serve_lyrics, threaded=True)
Thread(target=watch_parent, daemon=True).start()
Path(os.environ["YAP_LYRICA_READY"]).write_text(str(server.server_port), encoding="ascii")
server.serve_forever()
