"""Controlled HTML/CSS pagination and PDF generation."""
from importlib.resources import files
from ._serpentype import CacheStats, Diagnostic, FontRegistry, PreparedDocument, Renderer
from .compat import CSS, HTML, Document, FontConfiguration


def bundled_font_path(weight: int = 400) -> str:
    """Return the explicitly importable OFL-licensed Noto Sans TTF path."""
    name = "NotoSans-Bold.ttf" if weight == 700 else "NotoSans-Regular.ttf"
    return str(files("serpentype").joinpath("assets", name))


__all__ = ["CSS", "HTML", "Document", "FontConfiguration", "CacheStats", "Diagnostic", "FontRegistry", "PreparedDocument", "Renderer", "bundled_font_path"]
