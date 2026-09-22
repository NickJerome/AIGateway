"""Test the uploaded ZIP in a temporary real gateway; --live also calls Octopus."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[3]
PLUGIN = Path(__file__).resolve().parents[1]
MODEL = "qwen3.8-flash-next"
ADMIN = "octopus-isolated-test-admin"
ENDPOINTS = ("chat/completions", "responses", "messages")


def payload(endpoint, stream=False):
    data = {"model": MODEL, "stream": stream}
    if endpoint == "responses":
        data.update(input="Reply with exactly OK.", max_output_tokens=256)
    else:
        data.update(messages=[{"role": "user", "content": "Reply with exactly OK."}], max_tokens=256)
    return data


def usage_for(path):
    if path.endswith("chat/completions"):
        return {"prompt_tokens": 2, "completion_tokens": 4, "total_tokens": 6, "prompt_tokens_details": {"cached_tokens": 1}}
    if path.endswith("responses"):
        return {"input_tokens": 2, "output_tokens": 4, "total_tokens": 6, "input_tokens_details": {"cached_tokens": 1}}
    return {"input_tokens": 2, "output_tokens": 4, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 1}


def stream_tail(path):
    usage = usage_for(path)
    if path.endswith("chat/completions"):
        events = [{"usage": usage}, {"usage": usage}]
    elif path.endswith("responses"):
        events = [{"type": "response.completed", "response": {"usage": usage, "model": MODEL}}]
    else:
        events = [{"type": "message_start", "message": {"usage": {**usage, "output_tokens": 0}}},
                  {"type": "message_delta", "usage": {"output_tokens": 1}},
                  {"type": "message_delta", "usage": {"output_tokens": 4}}, {"type": "message_stop"}]
    return ("\n" + "".join("data: " + json.dumps(e) + "\r\n\r\n" for e in events) + "data: [DONE]\n\n").encode()


class Mock(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    requests = []
    release = threading.Event()

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        self.requests.append((self.path, dict(self.headers), body))
        if body.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(b'data: {"text":"OK"}\n\n')
            self.wfile.flush()
            self.release.wait(5)
            self.wfile.write(stream_tail(self.path)[1:])
            self.wfile.flush()
            self.close_connection = True
        else:
            status = 429 if body.get("test_error") else 200
            response = {"error": {"message": "limited"}} if status == 429 else {"model": body["model"], "usage": usage_for(self.path), "text": "OK"}
            raw = json.dumps(response).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.send_header("Retry-After", "2")
            self.end_headers()
            self.wfile.write(raw)


class Gateway:
    def __init__(self, directory):
        self.directory = Path(directory)
        config = self.directory / "gateway.toml"
        config.write_text(f'listen = "127.0.0.1:0"\ndata_dir = {json.dumps(str(self.directory / "data"))}\nthreads = 1\nadmin_token_env = "OCTOPUS_TEST_ADMIN"\n')
        binary = Path(os.environ.get("GATEWAY_BIN", ROOT / "gateway/target/debug/gateway"))
        self.log = (self.directory / "gateway.log").open("w")
        self.child = subprocess.Popen([str(binary), "run", "--config", str(config)], env={**os.environ, "OCTOPUS_TEST_ADMIN": ADMIN}, stdout=subprocess.PIPE, stderr=self.log, text=True)
        lines = queue.Queue()
        threading.Thread(target=lambda: [lines.put(line) for line in self.child.stdout], daemon=True).start()
        try:
            ready = lines.get(timeout=15)
            assert "gateway ready" in ready, "Gateway did not start"
            self.base = "http://" + ready.split("listen=", 1)[1].split()[0]
        except BaseException:
            self.close()
            raise

    def close(self):
        if self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait()
        self.log.close()

    def open(self, path, body=None, token=None, method=None, headers=None):
        headers = dict(headers or {})
        if token:
            headers["Authorization"] = "Bearer " + token
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(self.base + path, data=body, headers=headers, method=method)
        try:
            return urllib.request.urlopen(request, timeout=90)
        except urllib.error.HTTPError as error:
            return error

    def call(self, path, body=None, token=None, method=None, headers=None):
        with self.open(path, body, token, method, headers) as response:
            return response.status, response.read(), response.headers

    def configure(self, upstream, api_key):
        status, _, _ = self.call("/__gateway/plugins/octopus/config", {
            "vars": {"UPSTREAM": upstream, "MODEL": MODEL, "API_KEY": api_key},
            "trace_body": False,
        }, ADMIN, "PUT")
        assert status == 200, f"Configuration returned {status}"


def run(live=False):
    secret = os.environ.get("OCTOPUS_API_KEY")
    if live and not secret:
        raise SystemExit("Set OCTOPUS_API_KEY for --live (never written to the plugin ZIP).")
    subprocess.run(["python3", str(PLUGIN / "package.py")], check=True, stdout=subprocess.DEVNULL)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Mock)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="octopus-plugin-test-") as directory:
            gateway = Gateway(directory)
            try:
                status, _, _ = gateway.call("/__gateway/plugins", (PLUGIN / "octopus.zip").read_bytes(), ADMIN, "POST", {"Content-Type": "application/zip"})
                assert status == 201, f"Upload returned {status}"
                status, raw, _ = gateway.call("/__gateway/keys", {"name": "octopus-test"}, ADMIN, "POST")
                assert status == 201
                key = json.loads(raw)
                token = key["key"]
                gateway.configure(f"http://127.0.0.1:{server.server_port}", "upstream-test-secret")
                assert gateway.call("/octopus/v1/models")[0] == 401
                assert gateway.call("/octopus/v1/models", token=token)[0] == 200
                for endpoint in ENDPOINTS:
                    path = "/octopus/v1/" + endpoint
                    status, raw, _ = gateway.call(path, payload(endpoint), token, headers={"x-api-key": "client-must-not-leak", "cookie": "client-cookie"})
                    assert status == 200 and json.loads(raw)["text"] == "OK"
                    sent_path, sent_headers, sent_body = Mock.requests[-1]
                    sent_headers = {k.lower(): v for k, v in sent_headers.items()}
                    assert sent_path == "/v1/" + endpoint and sent_body == payload(endpoint)
                    assert sent_headers["authorization"] == "Bearer upstream-test-secret"
                    assert "cookie" not in sent_headers
                    if endpoint == "messages":
                        assert sent_headers["x-api-key"] == "upstream-test-secret"
                        assert sent_headers["anthropic-version"] == "2023-06-01"
                    else:
                        assert "x-api-key" not in sent_headers
                    Mock.release.clear()
                    started = time.monotonic()
                    try:
                        with gateway.open(path, payload(endpoint, True), token) as response:
                            assert response.status == 200
                            assert response.readline() == b'data: {"text":"OK"}\n'
                            assert time.monotonic() - started < 3, "SSE was buffered"
                            Mock.release.set()
                            assert response.read() == stream_tail('/v1/' + endpoint)
                    finally:
                        Mock.release.set()
                    print(f"mock {endpoint}: JSON + unbuffered SSE + credential isolation passed")
                for _ in range(50):
                    status, raw, _ = gateway.call("/__gateway/plugins/octopus/usage?groupBy=protocol", token=ADMIN)
                    totals = json.loads(raw)["rows"]
                    if sum(row["count"] for row in totals) == 6: break
                    time.sleep(.1)
                assert len(totals) == 3 and all(row["count"] == 2 for row in totals), totals
                for row in totals:
                    anthropic = row["group"]["protocol"] == "messages"
                    assert row["sums"]["input_tokens"] == (12 if anthropic else 4), row
                    assert row["sums"]["output_tokens"] == 8, row
                    assert row["sums"]["total_tokens"] == (20 if anthropic else 12), row
                status, raw, _ = gateway.call("/__gateway/plugins/octopus/logs?order=desc", token=ADMIN)
                assert any("upstream HTTP 200" in row["data"]["message"] for row in json.loads(raw)["rows"])
                assert Mock.requests[1][2]["stream_options"]["include_usage"] is True
                print("Core usage: six reports, protocol totals and console logs passed")
                base = "/octopus/v1/chat/completions"
                assert gateway.call(base, token=token)[0] == 405
                assert gateway.call(base, b"invalid", token, "POST")[0] == 400
                assert gateway.call(base, {"model": "not-supported"}, token)[0] == 400
                assert gateway.call(base, {"messages": []}, token)[0] == 200
                assert Mock.requests[-1][2]["model"] == MODEL
                status, raw, headers = gateway.call(base, {**payload("chat/completions"), "test_error": True}, token)
                assert status == 429 and json.loads(raw)["error"]["message"] == "limited" and headers["retry-after"] == "2"
                gateway.configure(f"http://127.0.0.1:{server.server_port}", "")
                assert gateway.call(base, payload("chat/completions"), token)[0] == 503
                print("mock validation, default model, upstream error/status, missing configuration: passed")
                if live:
                    gateway.configure("https://octopus.wangjerome.com", secret)
                    for endpoint in ENDPOINTS:
                        for streaming in (False, True):
                            started = time.monotonic()
                            status, raw, headers = gateway.call("/octopus/v1/" + endpoint, payload(endpoint, streaming), token)
                            assert status == 200, f"live {endpoint} stream={streaming}: HTTP {status}, body omitted"
                            if streaming:
                                assert "text/event-stream" in headers.get("content-type", "")
                                events = [json.loads(line[5:].strip()) for line in raw.decode().splitlines() if line.startswith("data:") and line[5:].strip() != "[DONE]"]
                                assert events and not any(event.get("type") == "error" or event.get("error") for event in events)
                                if endpoint == "chat/completions":
                                    assert b"[DONE]" in raw
                                    text = "".join(choice.get("delta", {}).get("content") or "" for event in events for choice in event.get("choices", []))
                                elif endpoint == "responses":
                                    assert any(event.get("type") == "response.completed" for event in events)
                                    text = "".join(event.get("delta", "") for event in events if event.get("type") == "response.output_text.delta")
                                else:
                                    assert any(event.get("type") == "message_stop" for event in events)
                                    text = "".join(event.get("delta", {}).get("text", "") for event in events if event.get("type") == "content_block_delta")
                            else:
                                data = json.loads(raw)
                                assert not data.get("error") and data.get("usage")
                                if endpoint == "chat/completions":
                                    text = data["choices"][0]["message"].get("content")
                                elif endpoint == "responses":
                                    text = "".join(part.get("text", "") for item in data.get("output", []) for part in item.get("content", []))
                                else:
                                    text = "".join(part.get("text", "") for part in data.get("content", []))
                            assert text and "OK" in text, f"live {endpoint}: missing expected output"
                            print(f"live {endpoint} {'SSE' if streaming else 'JSON'}: HTTP 200, output OK, {time.monotonic()-started:.2f}s")
                assert gateway.call("/__gateway/keys/" + key["id"], token=ADMIN, method="DELETE")[0] == 204
                assert gateway.call("/octopus/v1/models", token=token)[0] == 401
                print("Core Key revocation: passed")
            finally:
                gateway.close()
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    run(parser.parse_args().live)
