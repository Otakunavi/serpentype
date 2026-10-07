"""Check the release wheel's deterministic PDF serialization fingerprint."""
from __future__ import annotations

import hashlib

import serpentype


EXPECTED_SHA256 = "74fbc59ef7df3cf655ecbfa2ad9eb538142342d3860cf8d3dbb183e3070578a8"

fonts = serpentype.FontRegistry()
fonts.register_file(serpentype.bundled_font_path(), family="Noto Sans")
document = serpentype.Renderer(fonts=fonts).layout(
    "<p>Repeatable PDF 2026</p>",
    "@page { size:200pt 100pt; margin:10pt }",
)
actual = hashlib.sha256(document.to_pdf()).hexdigest()
if actual != EXPECTED_SHA256:
    raise SystemExit(f"deterministic PDF SHA256 mismatch: {actual} != {EXPECTED_SHA256}")
print(f"deterministic PDF SHA256: {actual}")
