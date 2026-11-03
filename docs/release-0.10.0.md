# Serpentype 0.10.0

Serpentype 0.10.0 fixes image rendering in nested inline-blocks for embedded raster and SVG data URIs, including `display: block` images used by notice templates. It also resolves default page margin-box fonts from the page context while preserving explicit font overrides.

The release adds regression coverage for image and font rendering, plus an optional end-to-end Russian/Kazakh notice PDF pipeline test. Install it with `python3 -m pip install ".[test-pdfpipe]"` and run `python3 -m unittest discover -s tests -p 'test_pdfpipe.py' -v`.
