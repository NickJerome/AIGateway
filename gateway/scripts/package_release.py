#!/usr/bin/env python3
"""Package reviewable runtime files and source separately, with SHA-256 manifests."""
import argparse
import hashlib
from pathlib import Path
import shutil
import tarfile
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--output-dir', default='dist')
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
output = (root / args.output_dir).resolve()
output.mkdir(parents=True, exist_ok=True)
name = 'aigateway-spec05-12-linux-x86_64'
with tempfile.TemporaryDirectory(prefix='gateway-package-') as temporary:
    package = Path(temporary) / name
    package.mkdir()
    shutil.copy2(root / 'gateway/target/release/gateway', package / 'gateway')
    shutil.copy2(root / 'gateway/gateway.example.toml', package / 'gateway.toml')
    (package / 'examples').mkdir()
    shutil.copy2(root / 'gateway/examples/codex-pool/codex-pool.zip', package / 'examples/codex-pool.zip')
    shutil.copy2(root / 'gateway/examples/codex-pool/README.md', package / 'examples/README.md')
    (package / 'docs').mkdir()
    shutil.copy2(root / 'docs/spec5-12-implementation.md', package / 'docs/spec5-12-implementation.md')
    shutil.copy2(root / 'README.md', package / 'docs/admin-and-development.md')
    evidence = package / 'reviews/spec5-12-validation'
    evidence.mkdir(parents=True)
    evidence_files = ['README.md', 'checks.json', 'build.json', 'environment.json',
                      'websocket-capacity.json', 'storage-capacity.json',
                      'full-tests.log', 'release-tests.log', 'fmt-check.log',
                      'clippy.log', 'release.log', 'plugin-build.log',
                      'do-init-final.log', 'do-oom-final.log', 'package-smoke.json']
    for filename in evidence_files:
        path = root / 'reviews/spec5-12-validation' / filename
        if path.exists():
            shutil.copy2(path, evidence / filename)
    for directory in ['final-capacity', 'release-capacity']:
        path = root / 'reviews/spec5-12-validation' / directory
        if path.exists():
            shutil.copytree(path, evidence / directory)
    shutil.copytree(root / 'reviews/spec5-12-validation/licenses', package / 'licenses')
    (package / 'README.md').write_text('''# AIGateway 本地交付包

Linux x86_64/glibc，QuickJS 和 SQLite 静态编入。运行不需要 Node/npm/Wrangler。

```sh
export GATEWAY_ADMIN_TOKEN='替换为自己的管理员令牌'
./gateway run --config gateway.toml
curl -H "Authorization: Bearer $GATEWAY_ADMIN_TOKEN" \\
  -H 'Content-Type: application/zip' --data-binary @examples/codex-pool.zip \\
  http://127.0.0.1:8080/__gateway/plugins
```

插件配置与登录见 examples/README.md；管理员 API 见 docs/admin-and-development.md；
模块边界见 docs/spec5-12-implementation.md；
测试、实际容量和未验证环境见 reviews/spec5-12-validation/README.md。

数据库默认 ./data/gateway.sqlite，安装代码/compiled 放在 ./data/plugins。
SIGTERM/SIGINT 按配置排空；重装取消旧工作。未进行外部部署或真实账户烟测。
''')
    entries = [f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(package)}'
               for p in sorted(package.rglob('*')) if p.is_file()]
    (package / 'SHA256SUMS').write_text('\n'.join(entries) + '\n')
    with tarfile.open(output / (name + '.tar.gz'), 'w:gz') as archive:
        archive.add(package, arcname=name)

source_name = 'aigateway-spec05-12-source'
paths = ['README.md', '.gitignore', '.github', 'docs', 'gateway/Cargo.toml',
         'gateway/Cargo.lock', 'gateway/build.rs', 'gateway/gateway.example.toml',
         'gateway/src', 'gateway/tests', 'gateway/scripts', 'gateway/examples']
def source_filter(info):
    if any(part in ('node_modules', 'target', '__pycache__', '.git') for part in Path(info.name).parts):
        return None
    return info
with tarfile.open(output / (source_name + '.tar.gz'), 'w:gz') as archive:
    for path in paths:
        archive.add(root / path, arcname=f'{source_name}/{path}', filter=source_filter)
for artifact in sorted(output.glob('aigateway-spec05-12-*.tar.gz')):
    digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
    artifact.with_suffix(artifact.suffix + '.sha256').write_text(f'{digest}  {artifact.name}\n')
    print(f'{artifact}: {artifact.stat().st_size} bytes {digest}')
