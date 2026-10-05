#!/usr/bin/env python3
"""upload-server.py <dir> [port] — accept `curl -T file http://10.0.2.2:PORT/name` from a QEMU guest and
store it in <dir> (loopback only, name restricted to a plain file name). Pulls screenshots out of a GL VM
whose monitor cannot screendump."""
import http.server, os, re, sys
d, port = os.path.abspath(sys.argv[1]), int(sys.argv[2]) if len(sys.argv) > 2 else 8124
class H(http.server.BaseHTTPRequestHandler):
    def do_PUT(self):
        name = os.path.basename(self.path)
        if not re.fullmatch(r"[A-Za-z0-9._-]+", name):
            self.send_response(400); self.end_headers(); return
        n = int(self.headers.get("Content-Length", 0))
        open(os.path.join(d, name), "wb").write(self.rfile.read(n))
        self.send_response(201); self.end_headers(); print("received", name, n, flush=True)
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
