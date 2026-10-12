# Serpentype 0.9.2

Serpentype 0.9.2 fixes inline SVG image rendering inside nested inline-blocks.
Images styled with `display: block` now render in the inline flow, restoring QR
codes in the supplied production notice templates.

The release adds a Rust regression test for nested block-display SVG images and
updates the production-fixture compatibility test to verify that the vector
image is present in the generated PDF.
