#!/usr/bin/env python3
"""Extract a local release, verify checksums, install a fixture, and drain it."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import os
import queue
import signal
import subprocess
import tarfile
import tempfile
import threading
import urllib.request
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--archive', required=True)
parser.add_argument('--output', required=True)
args = parser.parse_args()
with tempfile.TemporaryDirectory(prefix='gateway-release-smoke-') as tmp:
    directory = Path(tmp)
    with tarfile.open(args.archive) as archive:
        archive.extractall(directory, filter='data')
    package = next(directory.iterdir())
    verify = subprocess.run(['sha256sum', '--check', '--quiet', 'SHA256SUMS'],
                            cwd=package, capture_output=True, text=True, check=True)
    config = package / 'smoke.toml'
    config.write_text("listen='127.0.0.1:0'\ndata_dir='./smoke-data'\nthreads=1\nadmin_token_env='GATEWAY_SMOKE_ADMIN'\n")
    with (package / 'smoke.stderr').open('w+') as errors:
        process = subprocess.Popen(['./gateway', 'run', '--config', str(config)],
                                   cwd=package, stdout=subprocess.PIPE, stderr=errors,
                                   text=True, env={**os.environ, 'GATEWAY_SMOKE_ADMIN': 'local-smoke-fixture'})
        try:
            ready = queue.Queue()
            def wait_ready():
                for line in process.stdout:
                    if 'gateway ready' in line:
                        ready.put(line)
                        return
            threading.Thread(target=wait_ready, daemon=True).start()
            line = ready.get(timeout=15)
            base = 'http://' + line.split('listen=')[1].split()[0]
            fixture = io.BytesIO()
            with zipfile.ZipFile(fixture, 'w') as archive:
                archive.writestr('wrangler.toml', "name='smoke'\nmain='index.mjs'\nroutes=['/smoke/*']\n")
                archive.writestr('index.mjs', "export default {fetch(){return new Response('release-ok')}}")
            request = urllib.request.Request(base + '/__gateway/plugins',
                data=fixture.getvalue(), method='POST', headers={
                'Authorization': 'Bearer local-smoke-fixture', 'Content-Type': 'application/zip'})
            with urllib.request.urlopen(request, timeout=15) as response:
                assert response.status == 201
            with urllib.request.urlopen(base + '/smoke/hello', timeout=15) as response:
                assert response.read() == b'release-ok'
            process.send_signal(signal.SIGTERM)
            assert process.wait(timeout=35) == 0
            errors.seek(0)
            report = {'binary_sha256': hashlib.sha256((package / 'gateway').read_bytes()).hexdigest(),
                      'manifest_verified': True, 'zip_install_status': 201,
                      'http_body': 'release-ok', 'sigterm_exit_code': 0, 'stderr': errors.read()}
            Path(args.output).write_text(json.dumps(report, indent=2) + '\n')
            print(json.dumps(report, indent=2))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
