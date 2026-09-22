from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED
root = Path(__file__).resolve().parent
with ZipFile(root / 'codex.zip', 'w', ZIP_DEFLATED) as archive:
    for name in ('wrangler.toml', 'dist/index.mjs', 'HONO-LICENSE', 'CLIProxyAPI-LICENSE', 'NOTICE.md'):
        archive.write(root / name, name)
print(root / 'codex.zip')
