"""Package the dependency-free plugin for the gateway upload endpoint."""
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

root = Path(__file__).resolve().parent
with ZipFile(root / "octopus.zip", "w", ZIP_DEFLATED) as archive:
    for name in ("wrangler.toml", "index.mjs"):
        archive.write(root / name, name)
print(root / "octopus.zip")
