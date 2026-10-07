"""Check the release wheel's deterministic PDF serialization fingerprint."""
from __future__ import annotations

import hashlib

import serpentype


EXPECTED_SHA256 = "64750613774dc2e9ebaf383ab97993ecb717bd96b9cc5b26ef321593aa6b4e00"

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
