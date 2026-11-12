"""A host-run fake model endpoint on 127.0.0.1:argv[1]. Appends one JSON
line per request to argv[2]: method, path, and the credential headers it
received (fixture values only). Answers every request 400: pi gives up
at once, claude after six tries."""

import http.server
import json
import sys

LOG = sys.argv[2]
SEEN = ("authorization", "x-api-key", "chatgpt-account-id", "host", "user-agent")


class Handler(http.server.BaseHTTPRequestHandler):
    def handle_one(self):
        length = int(self.headers.get("Content-Length") or 0)
        if length:
            self.rfile.read(length)
        line = {
            "method": self.command,
            "path": self.path.split("?")[0],
            "headers": {k.lower(): v for k, v in self.headers.items() if k.lower() in SEEN},
        }
        with open(LOG, "a") as out:
            out.write(json.dumps(line) + "\n")
        body = b'{"type":"error","error":{"type":"invalid_request_error","message":"fixture"}}'
        self.send_response(400)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST = do_HEAD = handle_one

    def log_message(self, *args):
        pass


http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
