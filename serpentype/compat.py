"""HTML/CSS compatibility entry points backed by Serpentype's Rust renderer."""

from __future__ import annotations

import html as html_module
import base64
import mimetypes
import os
import re
from dataclasses import dataclass
from importlib.resources import files
from pathlib import Path
from urllib.parse import unquote, urljoin, urlsplit

from ._serpentype import FontRegistry, Renderer
from .resources import ResourceKind, ResourceLoader

_FONT_FACE = re.compile(r"@font-face\s*\{([^{}]*)\}", re.IGNORECASE | re.DOTALL)
_URL = re.compile(r"url\(\s*(['\"]?)(.*?)\1\s*\)", re.IGNORECASE | re.DOTALL)
_IMG = re.compile(r"<img\b[^>]*>", re.IGNORECASE | re.DOTALL)
_SRC = re.compile(
    r"\bsrc\s*=\s*(?:\"(?P<double>[^\"]*)\"|'(?P<single>[^']*)'|(?P<bare>[^\s>]+))",
    re.IGNORECASE | re.DOTALL,
)


def _read_source(*, string=None, filename=None, file_obj=None, encoding=None):
    supplied = sum(value is not None for value in (string, filename, file_obj))
    if supplied != 1:
        raise TypeError("provide exactly one of string, filename, or file_obj")
    source_name = None
    if filename is not None:
        data = Path(filename).read_bytes()
        inferred_base = Path(filename).resolve().parent
        source_name = os.fspath(filename)
    elif file_obj is not None:
        data = file_obj.read()
        name = getattr(file_obj, "name", None)
        inferred_base = Path(name).resolve().parent if name and Path(name).is_file() else None
        source_name = os.fspath(name) if name else None
    else:
        data = string
        inferred_base = None
    if isinstance(data, bytes):
        data = data.decode(encoding or "utf-8")
    if not isinstance(data, str):
        raise TypeError("HTML/CSS source must be str or bytes")
    return data, inferred_base, source_name


def _local_path(url: str, base_url, inferred_base=None) -> Path:
    url = html_module.unescape(url.strip())
    base = os.fspath(base_url) if base_url is not None else None
    if base:
        parsed_base = urlsplit(base)
        if parsed_base.scheme == "file":
            base_path = Path(unquote(parsed_base.path))
            if not base.endswith("/"):
                base_path = base_path.parent
        elif parsed_base.scheme:
            raise ValueError(f"non-local base_url is unsupported: {base}")
        else:
            base_path = Path(base)
            if base_path.is_file():
                base_path = base_path.parent
    else:
        base_path = inferred_base or Path.cwd()

    parsed = urlsplit(url)
    if parsed.scheme == "file":
        if parsed.netloc not in ("", "localhost"):
            raise ValueError(f"non-local file URL is unsupported: {url}")
        return Path(unquote(parsed.path))
    if parsed.scheme:
        raise ValueError(f"remote resource is unsupported: {url}")
    return (base_path / unquote(parsed.path)).resolve()


def _resource_url(url: str, base_url, inferred_base=None) -> str:
    if urlsplit(url).scheme:
        return url
    if base_url is not None:
        base = os.fspath(base_url)
        if urlsplit(base).scheme in ("http", "https", "file"):
            return urljoin(base, url)
        return str(_local_path(url, base, inferred_base))
    if inferred_base is not None:
        return str((inferred_base / url).resolve())
    return url


def _declaration(block: str, name: str):
    match = re.search(rf"(?:^|;)\s*{re.escape(name)}\s*:\s*([^;]+)", block, re.IGNORECASE)
    return match.group(1).strip() if match else None


@dataclass(frozen=True)
class _FontFace:
    source: Path | bytes
    family: str
    weight: int
    style: str


def _extract_font_faces(css: str, base_url, inferred_base, loader=None):
    faces = []

    def remove_face(match):
        block = match.group(1)
        family = _declaration(block, "font-family")
        source = _declaration(block, "src")
        if not family or not source:
            raise ValueError("@font-face requires font-family and src")
        urls = [candidate for _, candidate in _URL.findall(source)]
        if not urls:
            raise ValueError(f"@font-face has no url() source: {family}")
        candidates = [url for url in urls if Path(urlsplit(url).path).suffix.lower()
                      in {".ttf", ".otf", ".woff", ".woff2"}]
        if not candidates:
            raise ValueError(f"@font-face requires a TTF, OTF, WOFF or WOFF2 font: {family}")
        if loader is None:
            source = _local_path(candidates[0], base_url, inferred_base)
            if not source.is_file():
                raise FileNotFoundError(source)
        else:
            source = loader.load(_resource_url(candidates[0], base_url, inferred_base),
                                 ResourceKind.FONT).data
        weight_text = (_declaration(block, "font-weight") or "400").lower()
        weight = 700 if weight_text == "bold" else int(weight_text)
        style = (_declaration(block, "font-style") or "normal").lower()
        faces.append(_FontFace(source, family.strip("'\" "), weight, style))
        return ""

    return _FONT_FACE.sub(remove_face, css), tuple(faces)


class FontConfiguration:
    """Font registry shared by CSS and HTML, matching the app's call shape."""

    def __init__(self):
        self.registry = FontRegistry()
        regular = str(files("serpentype").joinpath("assets", "NotoSans-Regular.ttf"))
        bold = str(files("serpentype").joinpath("assets", "NotoSans-Bold.ttf"))
        self.registry.register_file(regular, family="Noto Sans")
        self.registry.register_file(bold, family="Noto Sans", weight=700)
        system_times = Path("/System/Library/Fonts/Supplemental")
        times_faces = (
            (system_times / "Times New Roman.ttf", 400, "normal"),
            (system_times / "Times New Roman Bold.ttf", 700, "normal"),
            (system_times / "Times New Roman Italic.ttf", 400, "italic"),
            (system_times / "Times New Roman Bold Italic.ttf", 700, "italic"),
        )
        if all(path.is_file() for path, _, _ in times_faces):
            for family in ("serif", "Times Newer Roman"):
                for path, weight, style in times_faces:
                    self.registry.register_file(
                        str(path), family=family, weight=weight, style=style
                    )
        else:
            for family in ("serif", "Times Newer Roman"):
                self.registry.register_file(regular, family=family)
                self.registry.register_file(bold, family=family, weight=700)

        for family in ("sans-serif",):
            self.registry.register_file(regular, family=family)
            self.registry.register_file(bold, family=family, weight=700)
        system_verdana = Path("/System/Library/Fonts/Supplemental")
        verdana_faces = (
            (system_verdana / "Verdana.ttf", 400, "normal"),
            (system_verdana / "Verdana Bold.ttf", 700, "normal"),
            (system_verdana / "Verdana Italic.ttf", 400, "italic"),
            (system_verdana / "Verdana Bold Italic.ttf", 700, "italic"),
        )
        if all(path.is_file() for path, _, _ in verdana_faces):
            for path, weight, style in verdana_faces:
                self.registry.register_file(
                    str(path), family="Inter", weight=weight, style=style
                )
        else:
            self.registry.register_file(regular, family="Inter")
            self.registry.register_file(bold, family="Inter", weight=700)

class CSS:
    """Stylesheet constructed from string, filename, or a readable file object."""

    def __init__(self, *, string=None, filename=None, file_obj=None, base_url=None,
                 font_config=None, encoding=None, resource_loader=None, url_fetcher=None):
        source, inferred_base, _ = _read_source(
            string=string, filename=filename, file_obj=file_obj, encoding=encoding
        )
        if resource_loader is not None and url_fetcher is not None:
            raise TypeError("provide resource_loader or url_fetcher")
        self.resource_loader = resource_loader or (
            ResourceLoader(callback=url_fetcher, allowed_schemes=("file", "data", "package", "memory", "http", "https"))
            if url_fetcher is not None else None
        )
        self.source = source
        self.text, self.font_faces = _extract_font_faces(
            source, base_url, inferred_base, self.resource_loader
        )
        self.base_url = base_url
        self.font_config = font_config or FontConfiguration()


class Document:
    """Prepared Serpentype document with pages and write_pdf()."""

    def __init__(self, prepared):
        self._prepared = prepared
        self.pages = tuple(range(prepared.page_count))
        self.warnings = prepared.warnings
        self.diagnostics = prepared.diagnostics
        self.overflow_count = prepared.overflow_count
        self.missing_glyphs = prepared.missing_glyphs
        self.unsupported_features = prepared.unsupported_features
        self.metadata = prepared.metadata

    @property
    def render_stats(self):
        return getattr(self._prepared, "render_stats", None)

    def write_pdf(self, target=None):
        data = bytes(self._prepared.to_pdf())
        if target is None:
            return data
        if hasattr(target, "write"):
            target.write(data)
        else:
            Path(target).write_bytes(data)
        return None


def _rewrite_file_images(source: str, base_url, inferred_base, loader=None):
    def rewrite_tag(match):
        tag = match.group()
        src_match = _SRC.search(tag)
        if not src_match:
            return tag
        group = next(name for name in ("double", "single", "bare") if src_match.group(name) is not None)
        src = html_module.unescape(src_match.group(group))
        if src.startswith("data:"):
            return tag
        if loader is not None:
            resource = loader.load(_resource_url(src, base_url, inferred_base),
                                   ResourceKind.IMAGE)
            mime = resource.media_type or mimetypes.guess_type(src)[0] or "application/octet-stream"
            replacement = f"data:{mime};base64,{base64.b64encode(resource.data).decode('ascii')}"
            start, end = src_match.span(group)
            return tag[:start] + replacement + tag[end:]
        elif src.startswith("file:"):
            path = _local_path(src, base_url, inferred_base)
        elif urlsplit(src).scheme:
            raise ValueError(f"remote image is unsupported: {src}")
        else:
            return tag
        start, end = src_match.span(group)
        return tag[:start] + str(path) + tag[end:]

    return _IMG.sub(rewrite_tag, source)


class HTML:
    """HTML entry point for the call forms used by cameral-control."""

    def __init__(self, *, string=None, filename=None, file_obj=None, base_url=None,
                 encoding=None, resource_loader=None, url_fetcher=None):
        self._source, self._inferred_base, self._source_name = _read_source(
            string=string, filename=filename, file_obj=file_obj, encoding=encoding
        )
        self.base_url = base_url
        if resource_loader is not None and url_fetcher is not None:
            raise TypeError("provide resource_loader or url_fetcher")
        self.resource_loader = resource_loader or (
            ResourceLoader(callback=url_fetcher, allowed_schemes=("file", "data", "package", "memory", "http", "https"))
            if url_fetcher is not None else None
        )

    def render(self, *, stylesheets=None, font_config=None, presentational_hints=False,
               limits=None):
        sheets = tuple(stylesheets or ())
        if any(not isinstance(sheet, CSS) for sheet in sheets):
            raise TypeError("stylesheets must contain CSS objects")
        config = font_config or next(
            (sheet.font_config for sheet in sheets if sheet.font_config is not None), None
        ) or FontConfiguration()
        for sheet in sheets:
            for face in sheet.font_faces:
                if isinstance(face.source, bytes):
                    config.registry.register_bytes(
                        face.source, family=face.family, weight=face.weight, style=face.style
                    )
                else:
                    config.registry.register_file(
                        str(face.source), family=face.family, weight=face.weight, style=face.style
                    )
        css_text = "\n".join(sheet.text for sheet in sheets)
        base_dir = (Path.cwd() if self.resource_loader is not None and self.base_url
                    and urlsplit(os.fspath(self.base_url)).scheme in ("http", "https")
                    else _local_path(".", self.base_url, self._inferred_base))
        html = _rewrite_file_images(self._source, self.base_url, self._inferred_base,
                                    self.resource_loader)
        prepared = Renderer(fonts=config.registry, base_dir=str(base_dir),
                            presentational_hints=presentational_hints,
                            source_name=self._source_name, limits=limits,
                            use_font_bbox_for_line_height=True).layout(
            html, css_text
        )
        return Document(prepared)

    def write_pdf(self, target=None, *, stylesheets=None, font_config=None,
                  presentational_hints=False, limits=None):
        return self.render(
            stylesheets=stylesheets,
            font_config=font_config,
            presentational_hints=presentational_hints,
            limits=limits,
        ).write_pdf(target)
