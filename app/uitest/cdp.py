"""drive the real freeplay window over the devtools protocol.

clickthrough.py and the rest run app/ui in a headless browser with a fake
bridge behind it, which is fast and catches most things and cannot see anything
that only happens with the real backend answering at real speed. webview2 will
open a devtools port if you ask it to:

    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222         target/release/freeplay-app.exe

then this attaches to the window that is actually on screen, with the real
games and the real tables. the websocket is written out by hand because
nothing is installed and one file beats a dependency.
"""
import base64, json, os, socket, struct, sys, urllib.request


def targets(port=9222):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json", timeout=5) as r:
        return json.load(r)


class Sock:
    def __init__(self, url):
        rest = url.split("://", 1)[1]
        hostport, path = rest.split("/", 1)
        host, port = hostport.split(":")
        self.s = socket.create_connection((host, int(port)), timeout=20)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.s.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            buf += self.s.recv(4096)
        assert b"101" in buf.split(b"\r\n")[0], buf[:200]
        self.buf = buf.split(b"\r\n\r\n", 1)[1]
        self.next_id = 0

    def _recv(self, n):
        while len(self.buf) < n:
            chunk = self.s.recv(65536)
            if not chunk:
                raise EOFError
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, text):
        data = text.encode()
        head = bytearray([0x81])
        mask = os.urandom(4)
        n = len(data)
        if n < 126:
            head.append(0x80 | n)
        elif n < 1 << 16:
            head.append(0x80 | 126)
            head += struct.pack(">H", n)
        else:
            head.append(0x80 | 127)
            head += struct.pack(">Q", n)
        head += mask
        self.s.sendall(bytes(head) + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def recv(self):
        while True:
            b0, b1 = self._recv(2)
            op, n = b0 & 0x0F, b1 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._recv(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._recv(8))[0]
            body = self._recv(n)
            if op == 1:
                return body.decode()
            if op == 8:
                raise EOFError("closed")

    def call(self, method, **params):
        self.next_id += 1
        mine = self.next_id
        self.send(json.dumps({"id": mine, "method": method, "params": params}))
        while True:
            msg = json.loads(self.recv())
            if msg.get("id") == mine:
                if "error" in msg:
                    raise RuntimeError(msg["error"])
                return msg.get("result", {})

    def js(self, expr, wait=True):
        r = self.call(
            "Runtime.evaluate",
            expression=expr,
            awaitPromise=wait,
            returnByValue=True,
        )
        if "exceptionDetails" in r:
            raise RuntimeError(r["exceptionDetails"].get("text", r["exceptionDetails"]))
        return r.get("result", {}).get("value")


def page(port=9222):
    """the main window, not the overlay and not the blank tab webview2 keeps."""
    found = [t for t in targets(port) if t.get("type") == "page"]
    for t in found:
        url = t.get("url", "")
        if "tauri.localhost" in url and "overlay" not in url:
            return Sock(t["webSocketDebuggerUrl"])
    # picking whatever came first landed on about:blank and every query came
    # back empty, which reads exactly like the app being broken
    raise SystemExit(
        "no freeplay window among %r" % [t.get("url") for t in found]
    )


if __name__ == "__main__":
    p = page()
    print(p.js("document.title + ' | ' + location.pathname"))
    print(p.js("[...document.querySelectorAll('#library-rail .rail-game')].map(r => r.textContent.trim().split('\\n')[0])"))
