"""Run a packaged Linux plugin against a local reference Xray and an echo server."""
import argparse
import base64
import contextlib
import gzip
import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import socket
import socketserver
import ssl
import subprocess
import tarfile
import tempfile
import threading
import time
from urllib.parse import urlencode


ACCOUNT = "00000000-0000-4000-8000-000000000001"
PASSWORD = b"test-password"


class Echo(socketserver.BaseRequestHandler):
    def handle(self):
        while data := self.request.recv(8192):
            self.request.sendall(data)


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def wait_port(process, number):
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError("Fixture service exited")
        try:
            with socket.create_connection(("127.0.0.1", number), .1):
                return
        except OSError:
            time.sleep(.03)
    raise TimeoutError("Fixture service did not start")


@contextlib.contextmanager
def background(command, **kwargs):
    with subprocess.Popen(command, **kwargs) as process:
        try:
            yield process
        finally:
            process.terminate()
            process.wait(timeout=10)


def rpc(process, method, params):
    process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n")
    process.stdin.flush()
    return json.loads(process.stdout.readline())["result"]


def submit(process, query, listen):
    link = f"vless://{ACCOUNT}@127.0.0.1:{listen}?" + urlencode(query)
    return rpc(process, "http", {"method": "PUT", "path": "/profile", "body": {"link": link}})


def settled(process):
    for _ in range(1200):
        result = rpc(process, "http", {"method": "GET", "path": "/profile", "body": None})["body"]
        if not result.get("pending"):
            return result
        time.sleep(.05)
    raise TimeoutError("VPN preparation timed out")


def profile(process, query, listen):
    submit(process, query, listen)
    result = settled(process)
    assert result.get("active") and not result.get("error"), (query.get("type"), result)


class AssetHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        name = self.path.removeprefix("/")
        self.server.requests.append(name)
        if name not in self.server.files or self.server.offline:
            self.send_error(404)
            return
        data = self.server.files[name].read_bytes()
        if self.server.corrupt:
            # A valid gzip of the expected ELF length, with the wrong executable content.
            raw = gzip.decompress(data)
            data = gzip.compress(bytes([raw[0] ^ 1]) + raw[1:], mtime=0)
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data[:16])
            self.server.resume.wait(10)
            self.wfile.write(data[16:])
        except (BrokenPipeError, ConnectionResetError):
            pass


def check_cache(plugin, directory, assets, catalog):
    def name(worker):
        return f"vless-{worker}-linux-{catalog['arch']}.gz"

    profile(plugin, {"type": "tcp"}, 1234)
    assert assets.requests == [name("stream")]
    saved = (directory/"profile.json").read_bytes()
    assets.offline = True
    profile(plugin, {"type": "tcp"}, 1234)
    assert assets.requests == [name("stream")]

    assets.offline = False
    assets.corrupt = True
    submit(plugin, {"type": "grpc"}, 1234)
    state = settled(plugin)
    assert state["error"] == "vpn_engine_download_failed" and state["active"]
    assert (directory/"profile.json").read_bytes() == saved
    assert not (directory/"engines/vless-http2").exists()
    assert not list((directory/"engines").glob("*.part"))
    assets.corrupt = False
    profile(plugin, {"type": "grpc"}, 1234)
    profile(plugin, {"type": "tcp"}, 1234)
    cached = directory/"engines/vless-http2"
    with cached.open("r+b") as file:
        file.write(b"bad")
    profile(plugin, {"type": "grpc"}, 1234)
    assert hashlib.sha256(cached.read_bytes()).hexdigest() == catalog["workers"]["vless-http2"]["sha256"]
    assert assets.requests.count(name("http2")) == 3

    assets.resume.clear()
    selection = {"type": "tcp", "security": "tls", "fp": "chrome"}
    result = submit(plugin, selection, 1234)
    assert result["status"] == 202 and result["body"]["pending"] and result["body"]["active"]
    assert submit(plugin, selection, 1234)["status"] == 202
    for _ in range(100):
        if name("xtls") in assets.requests:
            break
        time.sleep(.02)
    assert assets.requests.count(name("xtls")) == 1
    assert rpc(plugin, "http", {"method": "DELETE", "path": "/profile", "body": None})["body"] == {"active": False, "configured": False}
    assets.resume.set()
    assert not (directory/"engines/vless-xtls").exists()
    assert not settled(plugin)["active"]
    print("Selective download, offline reuse, checksum repair, rollback and cancellation passed", flush=True)


def connect(path, destination):
    client = socket.socket(socket.AF_UNIX)
    client.settimeout(12)
    client.connect(str(path))
    client.sendall(b"\x05\x01\x02")
    assert client.recv(2, socket.MSG_WAITALL) == b"\x05\x02"
    client.sendall(b"\x01\x07goodwin" + bytes([len(PASSWORD)]) + PASSWORD)
    assert client.recv(2, socket.MSG_WAITALL) == b"\x01\x00"
    client.sendall(b"\x05\x01\x00\x01\x7f\x00\x00\x01" + destination.to_bytes(2, "big"))
    assert client.recv(10, socket.MSG_WAITALL)[:2] == b"\x05\x00"
    return client


class SubscriptionHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        server = self.server
        if self.path == "/health":
            server.checks += 1
            self.send_response(204)
        elif self.path == "/subscription?token=secret":
            server.lookups += 1
            self.send_response(302)
            self.send_header("Location", "/profiles")
        elif self.path == "/profiles":
            if server.redirect:
                self.send_response(302)
                self.send_header("Location", server.links)
            else:
                self.send_response(200)
                self.end_headers()
                self.wfile.write(base64.b64encode(server.links.encode()))
                return
        elif self.path == "/delayed":
            server.started.set()
            server.resume.wait(10)
            self.send_response(302)
            self.send_header("Location", server.links)
        else:
            self.send_error(404)
            return
        self.end_headers()


def wait_until(condition, timeout=40):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(.05)
    raise TimeoutError("Subscription did not reach the expected state")


def check_subscription(directory, xray, env, echo):
    def controller():
        return background([directory / "bin/vpn"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)

    def request(plugin, method, path="/profile", body=None):
        return rpc(plugin, "http", {"method": method, "path": path, "body": body})

    def traffic():
        with connect(directory / "proxy", echo) as client:
            client.sendall(b"routed")
            assert client.recv(6, socket.MSG_WAITALL) == b"routed"

    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), SubscriptionHandler) as provider, contextlib.ExitStack() as resources:
        provider.checks = provider.lookups = 0
        provider.redirect = True
        provider.started, provider.resume = threading.Event(), threading.Event()
        threading.Thread(target=provider.serve_forever, daemon=True).start()
        resources.callback(provider.shutdown)
        base = f"http://127.0.0.1:{provider.server_port}"
        source = base + "/subscription?token=secret"
        (directory / ".goodwin-settings.json").write_text(json.dumps({"values": {"probe_url": base + "/health"}, "secrets": {}}))
        upstreams, links = [], []
        for transport in ("tcp", "grpc"):
            listen = port()
            config = {"inbounds": [{"listen": "127.0.0.1", "port": listen, "protocol": "vless",
                "settings": {"clients": [{"id": ACCOUNT}], "decryption": "none"},
                "streamSettings": {"network": transport}}], "outbounds": [{"protocol": "freedom",
                "settings": {"finalRules": [{"action": "allow", "ip": ["127.0.0.1/32"],
                "port": f"{provider.server_port},{echo}"}, {"action": "block"}]}}]}
            if transport == "grpc":
                config["inbounds"][0]["streamSettings"]["grpcSettings"] = {"serviceName": "Goodwin"}
            path = directory / f"subscription-{transport}.json"
            path.write_text(json.dumps(config))
            process = resources.enter_context(background([xray, "run", "-c", path], stdout=subprocess.DEVNULL))
            wait_port(process, listen)
            upstreams.append(process)
            query = {"type": transport}
            if transport == "grpc":
                query["serviceName"] = "Goodwin"
            links.append(f"vless://{ACCOUNT}@127.0.0.1:{listen}?" + urlencode(query))
        provider.links = links[0]
        profile_path = directory / "profile.json"
        with controller() as plugin:
            rpc(plugin, "initialize", {"apiVersion": 1})
            assert request(plugin, "PUT", body={"link": source, "intervalSeconds": 0})["status"] == 400
            request(plugin, "PUT", body={"link": source, "intervalSeconds": 5})
            assert settled(plugin) == {"active": True, "configured": True, "intervalSeconds": 5}
            assert json.loads(profile_path.read_text())["resolved"] == links[0]
            traffic()
            provider.redirect = False
            provider.links = links[1]
            wait_until(lambda: provider.checks >= 2)
            assert provider.lookups == 1
            saved = profile_path.read_bytes()
            upstreams[0].terminate()
            upstreams[0].wait(timeout=5)
            # The server still accepts TCP on this port but rejects the wrong VLESS UUID.
            wrong = links[1].replace(ACCOUNT, ACCOUNT[:-1] + "2")
            provider.links = wrong
            wait_until(lambda: request(plugin, "GET")["body"].get("error") == "vpn_probe_failed")
            assert profile_path.read_bytes() == saved
            provider.links = wrong + "\n" + links[1]
            wait_until(lambda: json.loads(profile_path.read_text())["resolved"] == links[1])
            traffic()
            assert request(plugin, "PUT", "/monitoring", {"intervalSeconds": 3600})["body"]["intervalSeconds"] == 3600
            lookups = provider.lookups
            assert lookups == 3
        provider.links = "subscription unavailable"
        with controller() as plugin:
            rpc(plugin, "initialize", {"apiVersion": 1})
            assert settled(plugin) == {"active": True, "configured": True, "intervalSeconds": 3600}
            assert provider.lookups == lookups
            traffic()
            saved = profile_path.read_bytes()
            request(plugin, "PUT", body={"link": source})
            assert settled(plugin)["error"] == "vpn_subscription_failed"
            assert profile_path.read_bytes() == saved
            traffic()
            provider.links = links[1]
            assert request(plugin, "PUT", body={"link": base + "/delayed"})["status"] == 202
            assert provider.started.wait(3)
            assert request(plugin, "DELETE")["body"] == {"active": False, "configured": False}
            provider.resume.set()
            assert settled(plugin) == {"active": False, "configured": False}
            assert json.loads(profile_path.read_text()) is None
        print("Subscription health checks, refresh, verified rotation, offline restore and cancellation passed", flush=True)


def exercise(xray, directory, controller, cover, echo):
    keygen = subprocess.check_output([xray, "x25519"], text=True).splitlines()
    private, public = [line.split(":", 1)[1].strip() for line in keygen[:2]]
    pairs = subprocess.check_output([xray, "vlessenc"], text=True)
    decrypt, encrypt = re.findall(r'"(?:decryption|encryption)":\s*"([^"]+)"', pairs)[:2]
    reality = {"security": "reality", "realitySettings": {"target": f"127.0.0.1:{cover}",
              "serverNames": ["localhost"], "privateKey": private, "shortIds": ["aabb"]}}
    identity = {"security": "reality", "pbk": public, "sni": "localhost", "sid": "aabb", "fp": "chrome"}
    for name, stream, query in [
        ("tcp", {"network": "raw"}, {"type": "tcp"}),
        ("ws", {"network": "ws", "wsSettings": {"path": "/socket"}}, {"type": "ws", "path": "/socket"}),
        ("upgrade", {"network": "httpupgrade", "httpupgradeSettings": {"path": "/up"}}, {"type": "httpupgrade", "path": "/up"}),
        ("grpc", {"network": "grpc", "grpcSettings": {"serviceName": "Goodwin"}}, {"type": "grpc", "serviceName": "Goodwin"}),
        ("xhttp", {"network": "xhttp", "xhttpSettings": {"path": "/x", "mode": "stream-one"}}, {"type": "xhttp", "path": "/x", "mode": "stream-one"}),
        ("reality-xhttp", {"network": "xhttp", "xhttpSettings": {"path": "/r", "mode": "stream-one"}, **reality},
         {"type": "xhttp", **identity, "path": "/r", "mode": "auto"}),
        ("reality-vision", {"network": "raw", **reality}, {"type": "tcp", **identity, "flow": "xtls-rprx-vision"}),
        ("encrypted", {"network": "raw"}, {"type": "raw", "encryption": encrypt}),
    ]:
        listen = port()
        account = {"id": ACCOUNT}
        if "flow" in query:
            account["flow"] = query["flow"]
        settings = {"clients": [account], "decryption": decrypt if "encryption" in query else "none"}
        config = {"inbounds": [{"listen": "127.0.0.1", "port": listen, "protocol": "vless",
                  "settings": settings, "streamSettings": stream}], "outbounds": [{"protocol": "freedom",
                  "settings": {"finalRules": [{"action": "allow", "ip": ["127.0.0.1/32"],
                                              "port": f"{cover},{echo}"}, {"action": "block"}]}}]}
        config_path = directory / "server.json"
        config_path.write_text(json.dumps(config))
        with background([xray, "run", "-c", config_path], stdout=subprocess.DEVNULL) as server:
            wait_port(server, listen)
            profile(controller, query, listen)
            if name in ("tcp", "ws", "upgrade"):
                expected = "vless-stream"
            elif name in ("grpc", "xhttp"):
                expected = "vless-http2"
            else:
                expected = "vless-xtls"
            children = Path(f"/proc/{controller.pid}/task/{controller.pid}/children").read_text().split()
            assert len(children) == 1, (name, children)
            args = Path(f"/proc/{children[0]}/cmdline").read_bytes().split(b"\0")
            assert any(Path(os.fsdecode(arg)).name == expected for arg in args), (name, args[:2])
            with connect(directory / "proxy", echo) as client:
                payload = os.urandom(65537)
                client.sendall(payload)
                with client.makefile("rb") as stream:
                    assert stream.read(len(payload)) == payload, name
            if name == "reality-vision":
                context = ssl.create_default_context(cafile=directory / "cert.pem")
                with connect(directory / "proxy", cover) as client, context.wrap_socket(client, server_hostname="localhost") as tls:
                    tls.sendall(b"GET / HTTP/1.0\r\n\r\n")
                    assert tls.recv(1024).startswith(b"HTTP/1.0 200"), name
                # The monitor verifies the certificate even over a working tunnel.
                probe = {"Probe": {"socket": str(directory / "proxy"), "password": PASSWORD.decode(),
                                   "url": f"https://127.0.0.1:{cover}/"}}
                untrusted = subprocess.run([directory / "bin/vpn-fetch", "--subscription"],
                                          input=json.dumps(probe), text=True, capture_output=True, timeout=30)
                assert untrusted.returncode != 0 and "certificate" in untrusted.stderr.lower(), untrusted.stderr
            print(f"{name}: round trip passed", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("xray", type=Path)
    args = parser.parse_args()
    xray = args.xray.resolve()
    with tempfile.TemporaryDirectory() as prefix, socketserver.ThreadingTCPServer(("127.0.0.1", 0), Echo) as echo:
        directory = Path(prefix)
        with tarfile.open(args.package) as archive:
            archive.extractall(directory, filter="data")
        assert not list((directory/"bin").glob("vless-*"))
        manifest = directory/"bin/engines.json"
        catalog = json.loads(manifest.read_text())
        for binary in (directory / "bin").iterdir():
            binary.chmod(0o700)
        threading.Thread(target=echo.serve_forever, daemon=True).start()
        cover = port()
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost",
                        "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1", "-keyout", directory/"key.pem", "-out", directory/"cert.pem",
                        "-days", "1"], check=True, stderr=subprocess.DEVNULL)
        env = {**os.environ, "GOODWIN_PLUGIN_DATA": prefix, "GOODWIN_VPN_SOCKET": str(directory/"proxy"),
               "GOODWIN_VPN_PASSWORD": PASSWORD.decode()}
        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), AssetHandler) as assets:
            assets.files = {p.name: p for p in args.package.parent.glob("vless-*-linux-*.gz")}
            assets.requests = []
            assets.offline = assets.corrupt = False
            assets.resume = threading.Event()
            assets.resume.set()
            catalog["base_url"] = f"http://127.0.0.1:{assets.server_port}"
            manifest.write_text(json.dumps(catalog))
            threading.Thread(target=assets.serve_forever, daemon=True).start()
            with background(["openssl", "s_server", "-accept", f"127.0.0.1:{cover}", "-cert", directory/"cert.pem",
                         "-key", directory/"key.pem", "-www", "-alpn", "h2,http/1.1"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as tls, \
             background([directory/"bin/vpn"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env) as plugin:
                wait_port(tls, cover)
                registration = rpc(plugin, "initialize", {"apiVersion": 1})
                for route in registration["routes"]:
                    assert not route.get("password_required"), route
                assert not assets.requests
                check_cache(plugin, directory, assets, catalog)
                exercise(xray, directory, plugin, cover, echo.server_address[1])
            before = list(assets.requests)
            assets.offline = True
            with background([directory/"bin/vpn"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env) as plugin:
                rpc(plugin, "initialize", {"apiVersion": 1})
                assert settled(plugin) == {"active": True, "configured": True}
                assert assets.requests == before
                state = rpc(plugin, "http", {"method": "DELETE", "path": "/profile", "body": None})
                assert state["body"] == {"active": False, "configured": False}
                assert not Path(f"/proc/{plugin.pid}/task/{plugin.pid}/children").read_text().strip()
            check_subscription(directory, xray, env, echo.server_address[1])
            assets.shutdown()
        echo.shutdown()


if __name__ == "__main__":
    main()
