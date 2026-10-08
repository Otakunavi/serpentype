# Serpentype 0.7.0

## Current API correction

The original 0.7.0 implementation delegated `HTML`/`CSS` calls to WeasyPrint when requested. That integration has been removed from the production API. `HTML` and `CSS` now always use Serpentype's Rust renderer; WeasyPrint remains available only through the optional `test-weasy` dependency as a visual reference.

## Original release notes

This release adds an exact-output mode for the WeasyPrint-shaped Python API. Install the optional dependency with:

```sh
python -m pip install 'serpentype[weasy-compat]'
```

The extra pins WeasyPrint 64.1. `HTML` and `CSS` use Serpentype's Rust renderer by default for speed. Pass `backend="weasyprint"` to select exact WeasyPrint compatibility or `backend="serpentype"` to require the Rust renderer. Direct `Renderer` calls continue to use the Rust implementation. The Rust renderer supports a controlled HTML/CSS subset and does not claim pixel identity with WeasyPrint.

The delegated backend preserves WeasyPrint's own rendering and PDF behavior for the supplied compatibility templates. The fixture pack completed with 38 successful render jobs; all 19 PDFs match the WeasyPrint 64.1 baseline by SHA-256 and byte length across 353 pages, including the 2000-row table. Verification ran on macOS arm64 with CPython 3.13. The delegated renderer checks a configured page limit after layout and cannot enforce custom non-page `RenderLimits` or resource-loader policies. WeasyPrint's platform-specific native runtime dependencies must be installed on the host.

Serpentype is AI-generated software, provided as-is and used at the user's own risk. Review and validate generated documents before relying on them.
