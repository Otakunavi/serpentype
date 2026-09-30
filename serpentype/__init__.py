"""Controlled HTML/CSS pagination and PDF generation."""
from importlib.resources import files
from ._serpentype import (CacheStats, CancelToken, Diagnostic, FontRegistry,
                          MissingGlyphError,
                          PreparedDocument, RenderCancelledError, RenderLimitError,
                          RenderLimits, RenderStats, Renderer, capabilities)
from .compat import CSS, HTML, Document, FontConfiguration
from .resources import (Resource, ResourceKind, ResourceLoader, ResourceLimitError,
                        ResourceCancelledError)


def bundled_font_path(weight: int = 400) -> str:
    """Return the explicitly importable OFL-licensed Noto Sans TTF path."""
    name = "NotoSans-Bold.ttf" if weight == 700 else "NotoSans-Regular.ttf"
    return str(files("serpentype").joinpath("assets", name))


__all__ = ["CSS", "HTML", "Document", "FontConfiguration", "Resource", "ResourceKind", "ResourceLoader", "ResourceLimitError", "ResourceCancelledError", "CacheStats", "CancelToken", "Diagnostic", "FontRegistry", "MissingGlyphError", "PreparedDocument", "RenderCancelledError", "RenderLimitError", "RenderLimits", "RenderStats", "Renderer", "bundled_font_path", "capabilities"]
