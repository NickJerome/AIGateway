from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED
root=Path(__file__).resolve().parent
with ZipFile(root/'codex-pool.zip','w',ZIP_DEFLATED) as out:
    for path in ('wrangler.toml','dist/index.mjs','HONO-LICENSE'):
        out.write(root/path,path)
print(root/'codex-pool.zip')
