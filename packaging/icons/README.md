# Application icons

Generated from the synofurb brand artwork in `assets/brand/` (see `assets/brand/README.md`): `python3 assets/brand/build.py && python3 assets/brand/export.py`.
File names are referenced by `packaging/Packager.toml`; keep them. `icon.icns` uses the macOS artwork (with the system margin), the PNGs and `icon.ico` are full-bleed.
Note: `packaging/make-icons.py` still generates the old placeholder set and would overwrite these files; do not run it.
