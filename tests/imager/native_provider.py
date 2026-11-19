"""Exercise the AI service editor against a loopback HTTP fixture in an isolated native session."""
import argparse
import json
import os
import secrets
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from tempfile import TemporaryDirectory
from threading import Thread

from native_driver import launch, wait


class ProviderHandler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def respond(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def authorized(self):
        if self.headers.get("Authorization") == "Bearer " + self.server.api_key:
            return True
        self.respond(401, {"error": "missing fixture credential"})
        return False

    def do_GET(self):
        if self.authorized():
            self.server.calls.append(self.path)
            self.respond(200 if self.path == "/v1/models" else 404, {"data": [{"id": "qa-model"}]})

    def do_POST(self):
        if not self.authorized():
            return
        self.server.calls.append(self.path)
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path != "/v1/chat/completions" or request.get("model") != "qa-model":
            self.respond(400, {"error": "unexpected request"})
        elif self.server.mode == "limited":
            self.respond(429, {"error": "fixture rate limit"})
        elif self.server.mode == "error-body":
            self.respond(200, {"error": {"message": "fixture application error"}})
        elif self.server.mode == "empty":
            self.respond(200, {"choices": []})
        else:
            self.respond(200, {"choices": [{"message": {"role": "assistant", "content": "OK"}}]})


def fill(web, label, value):
    web.js("""const row = [...document.querySelectorAll('.im-ai-body label')]
        .find(e => e.firstChild?.textContent.trim() === arguments[0]);
        const input = row.querySelector('input');
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,arguments[1]);
        input.dispatchEvent(new Event('input',{bubbles:true}));""", label, value)


def run(binary, output):
    output.mkdir(parents=True, exist_ok=True)
    checks = []
    passed = False
    with ThreadingHTTPServer(("127.0.0.1", 0), ProviderHandler) as server:
        server.api_key = secrets.token_hex(16)
        server.calls = []
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            with launch(binary) as (web, _):
                web.click(".imager-app-tabs button", "连接 AI")
                web.click(".im-ai-rail button", "AI 服务")
                web.shown("新建服务连接")
                fill(web, "连接名称", "Native QA")
                fill(web, "Base URL", f"http://127.0.0.1:{server.server_port}/v1")
                fill(web, "运行模型", "qa-model")
                fill(web, "API Key", server.api_key)
                web.click(".im-ai-body button", "保存连接")
                web.shown("编辑 Native QA")
                cases = [
                    ("ok", "available"), ("limited", "rate_limited"),
                    ("error-body", "invalid_response"), ("empty", "invalid_response"),
                    ("recovered", "available"),
                ]
                for mode, status in cases:
                    server.mode = mode
                    count = len(server.calls)
                    web.click(".im-ai-body button", "测试连接")
                    wait(lambda: len(server.calls) >= count + 2, "native HTTP requests")
                    web.shown("连接测试通过" if status == "available" else "连接测试失败")
                    web.shown(status)
                    assert server.calls[count:] == ["/v1/models", "/v1/chat/completions"]
                    assert web.ipc("supervisor_list_providers", {})[0]["lastTestStatus"] == status
                    assert server.api_key not in web.js("return document.body.innerText")
                    checks.append({"response": mode, "savedStatus": status})
                    web.shot(output / (mode + ".png"))
                web.click(".im-ai-rail button", "策略与安全")
                web.click(".im-ai-rail button", "AI 服务")
                web.shown("编辑 Native QA")
                web.shown("available")
                assert web.js("return document.querySelector('.im-ai-body input[type=password]').value === ''")
                saved = web.ipc("supervisor_list_providers", {})
                assert len(saved) == 1 and saved[0]["model"] == "qa-model" and saved[0]["hasApiKey"]
                assert server.api_key not in json.dumps(saved)
                passed = True
        finally:
            server.shutdown()
            report = {"passed": passed, "checks": checks, "requests": server.calls}
            (output / "provider.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf8")


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("binary", type=Path)
    cli.add_argument("output", type=Path)
    cli.add_argument("--private-session", action="store_true", help=argparse.SUPPRESS)
    args = cli.parse_args()
    if not args.private_session:
        subprocess.run(["dbus-run-session", "--", sys.executable, __file__, "--private-session",
                        str(args.binary), str(args.output)], check=True)
    else:
        with TemporaryDirectory(prefix="imager-qa-keyring-") as folder:
            for key, name in [("XDG_DATA_HOME","data"), ("XDG_RUNTIME_DIR","runtime")]:
                path = Path(folder) / name
                path.mkdir(mode=0o700)
                os.environ[key] = str(path)
            with subprocess.Popen(["gnome-keyring-daemon","--foreground","--unlock",
                                   "--components=secrets","--control-directory",folder+"/control"],
                                  stdin=subprocess.PIPE, stdout=subprocess.DEVNULL) as daemon:
                try:
                    daemon.stdin.write(secrets.token_hex(16).encode())
                    daemon.stdin.close()
                    run(args.binary, args.output)
                finally:
                    daemon.terminate()
