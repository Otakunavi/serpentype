# Serpentype 0.11.0

- Paints layered SVG and raster `@page` backgrounds on first, named, and landscape pages.
- Renders SVG QR placeholders declared as `image/svg` or `image/png`. Invalid inline images produce a diagnostic, and document rendering continues within the configured limits.
- Keeps a flex row together when the row fits on the next page. Supports explicit margin-box widths, line heights, and page margin overrides.
- Compares Russian and Kazakh draft and signed notices against WeasyPrint 64.1, including both watermark layers, footers, and QR payloads.
