"""Check the release wheel's deterministic PDF serialization fingerprint."""
from __future__ import annotations

import hashlib

import serpentype


EXPECTED_SHA256 = "a8356d1a9a03a38c5dd1c970f1c74955c7f8c753fda18db330092d8daf38bfba"

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
