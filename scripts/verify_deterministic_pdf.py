"""Check the release wheel's deterministic PDF serialization fingerprint."""
from __future__ import annotations

import hashlib

import serpentype


EXPECTED_SHA256 = "2df1225a1772d9920d1059a3999f4b5cca511ad85f65fc82f4f17443bfd38a9a"

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
