"""WeasyPrint-shaped Python entry points used by cameral-control.

This adapter preserves the callers' object lifecycle and output types. Rendering
still uses Serpentype's documented HTML/CSS subset.
"""

from __future__ import annotations

import base64
import html as html_module
import os
import re
from dataclasses import dataclass
from importlib.resources import files
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.parse import unquote, urlsplit
from xml.etree import ElementTree

from ._serpentype import FontRegistry, Renderer


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
    if filename is not None:
        data = Path(filename).read_bytes()
        inferred_base = Path(filename).resolve().parent
    elif file_obj is not None:
        data = file_obj.read()
        name = getattr(file_obj, "name", None)
        inferred_base = Path(name).resolve().parent if name and Path(name).is_file() else None
    else:
        data = string
        inferred_base = None
    if isinstance(data, bytes):
        data = data.decode(encoding or "utf-8")
    if not isinstance(data, str):
        raise TypeError("HTML/CSS source must be str or bytes")
    return data, inferred_base


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


def _declaration(block: str, name: str):
    match = re.search(rf"(?:^|;)\s*{re.escape(name)}\s*:\s*([^;]+)", block, re.IGNORECASE)
    return match.group(1).strip() if match else None


@dataclass(frozen=True)
class _FontFace:
    path: Path
    family: str
    weight: int
    style: str


def _extract_font_faces(css: str, base_url, inferred_base):
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
        candidates = [url for url in urls if Path(urlsplit(url).path).suffix.lower() in {".ttf", ".otf"}]
        if not candidates:
            raise ValueError(f"@font-face requires a local TTF-outline font: {family}")
        path = _local_path(candidates[0], base_url, inferred_base)
        if not path.is_file():
            raise FileNotFoundError(path)
        weight_text = (_declaration(block, "font-weight") or "400").lower()
        weight = 700 if weight_text == "bold" else int(weight_text)
        style = (_declaration(block, "font-style") or "normal").lower()
        faces.append(_FontFace(path, family.strip("'\" "), weight, style))
        return ""

    return _FONT_FACE.sub(remove_face, css), tuple(faces)


class FontConfiguration:
    """Font registry shared by CSS and HTML, matching the app's call shape."""

    def __init__(self):
        self.registry = FontRegistry()
        regular = str(files("serpentype").joinpath("assets", "NotoSans-Regular.ttf"))
        bold = str(files("serpentype").joinpath("assets", "NotoSans-Bold.ttf"))
        for family in ("Noto Sans", "serif", "sans-serif", "Times Newer Roman", "Inter"):
            self.registry.register_file(regular, family=family)
            self.registry.register_file(bold, family=family, weight=700)


class CSS:
    """Stylesheet constructed from string, filename, or a readable file object."""

    def __init__(self, *, string=None, filename=None, file_obj=None, base_url=None,
                 font_config=None, encoding=None):
        source, inferred_base = _read_source(
            string=string, filename=filename, file_obj=file_obj, encoding=encoding
        )
        self.text, self.font_faces = _extract_font_faces(source, base_url, inferred_base)
        self.base_url = base_url
        self.font_config = font_config


class Document:
    """Prepared document with WeasyPrint-style pages and write_pdf()."""

    def __init__(self, prepared):
        self._prepared = prepared
        self.pages = tuple(range(prepared.page_count))
        self.warnings = prepared.warnings

    def write_pdf(self, target=None):
        data = bytes(self._prepared.to_pdf())
        if target is None:
            return data
        if hasattr(target, "write"):
            target.write(data)
        else:
            Path(target).write_bytes(data)
        return None


def _materialize_data_images(source: str, directory: Path, base_url, inferred_base):
    count = 0

    def rewrite_tag(match):
        nonlocal count
        tag = match.group()
        src_match = _SRC.search(tag)
        if not src_match:
            return tag
        group = next(name for name in ("double", "single", "bare") if src_match.group(name) is not None)
        src = html_module.unescape(src_match.group(group))
        if src.startswith("data:"):
            header, separator, payload = src.partition(",")
            if not separator or not header.lower().endswith(";base64"):
                raise ValueError("only base64 data: images are supported")
            mime = header[5:-7].lower()
            suffix = {
                "image/png": ".png", "image/jpeg": ".jpg", "image/jpg": ".jpg",
                "image/svg+xml": ".png",
            }.get(mime)
            if suffix is None:
                raise ValueError(f"unsupported data: image type: {mime}")
            data = base64.b64decode(payload, validate=True)
            count += 1
            path = directory / f"image-{count}{suffix}"
            if mime == "image/svg+xml":
                _rasterize_simple_svg(data, path)
            else:
                path.write_bytes(data)
        elif src.startswith("file:"):
            path = _local_path(src, base_url, inferred_base)
        elif urlsplit(src).scheme:
            raise ValueError(f"remote image is unsupported: {src}")
        else:
            return tag
        start, end = src_match.span(group)
        return tag[:start] + str(path) + tag[end:]

    return _IMG.sub(rewrite_tag, source)


def _rasterize_simple_svg(data: bytes, destination: Path):
    """Rasterize the line/text SVG QR placeholders used by cameral-control."""
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError as error:
        raise RuntimeError("SVG QR placeholders need Pillow; install serpentype[weasy-compat]") from error

    root = ElementTree.fromstring(data)
    if root.tag.rsplit("}", 1)[-1] != "svg":
        raise ValueError("data:image/svg+xml must contain an SVG root")
    width = int(float(root.attrib.get("width", "0")))
    height = int(float(root.attrib.get("height", "0")))
    if not 0 < width <= 2048 or not 0 < height <= 2048:
        raise ValueError("unsupported SVG dimensions")
    scale = 2
    image = Image.new("RGB", (width * scale, height * scale), "white")
    draw = ImageDraw.Draw(image)
    regular = str(files("serpentype").joinpath("assets", "NotoSans-Regular.ttf"))
    bold = str(files("serpentype").joinpath("assets", "NotoSans-Bold.ttf"))

    def coordinate(value, span):
        value = str(value)
        return float(value[:-1]) * span / 100 if value.endswith("%") else float(value)

    def visit(element, stroke="black", stroke_width=1.0, fill="black", anchor="start"):
        tag = element.tag.rsplit("}", 1)[-1]
        stroke = element.attrib.get("stroke", stroke)
        stroke_width = float(element.attrib.get("stroke-width", stroke_width))
        fill = element.attrib.get("fill", fill)
        anchor = element.attrib.get("text-anchor", anchor)
        if tag in {"svg", "g"}:
            for child in element:
                visit(child, stroke, stroke_width, fill, anchor)
        elif tag == "line":
            xy = tuple(
                coordinate(element.attrib[key], width if key.startswith("x") else height) * scale
                for key in ("x1", "y1", "x2", "y2")
            )
            draw.line(xy, fill=stroke, width=max(1, round(stroke_width * scale)))
        elif tag == "rect":
            x = coordinate(element.attrib.get("x", 0), width) * scale
            y = coordinate(element.attrib.get("y", 0), height) * scale
            w = coordinate(element.attrib["width"], width) * scale
            h = coordinate(element.attrib["height"], height) * scale
            draw.rectangle((x, y, x + w, y + h), fill=fill if fill != "none" else None,
                           outline=stroke if stroke != "none" else None,
                           width=max(1, round(stroke_width * scale)))
        elif tag == "text":
            x = coordinate(element.attrib.get("x", 0), width) * scale
            y = coordinate(element.attrib.get("y", 0), height) * scale
            horizontal = "m" if anchor == "middle" else "l"
            for child in element:
                if child.tag.rsplit("}", 1)[-1] != "tspan":
                    raise ValueError("unsupported SVG text child")
                x = coordinate(child.attrib.get("x", element.attrib.get("x", 0)), width) * scale
                if "y" in child.attrib:
                    y = coordinate(child.attrib["y"], height) * scale
                y += float(child.attrib.get("dy", 0)) * scale
                size = round(float(child.attrib.get("font-size", 12)) * scale)
                font_path = bold if child.attrib.get("font-weight") in {"bold", "700"} else regular
                font = ImageFont.truetype(font_path, size)
                draw.text((x, y), "".join(child.itertext()), font=font,
                          fill=child.attrib.get("fill", fill), anchor=horizontal + "s")
        else:
            raise ValueError(f"unsupported SVG element: {tag}")

    visit(root)
    image.resize((width, height), Image.Resampling.LANCZOS).save(destination, format="PNG")


class HTML:
    """HTML entry point for the call forms used by cameral-control."""

    def __init__(self, *, string=None, filename=None, file_obj=None, base_url=None, encoding=None):
        self._source, self._inferred_base = _read_source(
            string=string, filename=filename, file_obj=file_obj, encoding=encoding
        )
        self.base_url = base_url

    def render(self, *, stylesheets=None, font_config=None, presentational_hints=False):
        sheets = tuple(stylesheets or ())
        if any(not isinstance(sheet, CSS) for sheet in sheets):
            raise TypeError("stylesheets must contain CSS objects")
        config = font_config or next(
            (sheet.font_config for sheet in sheets if sheet.font_config is not None), None
        ) or FontConfiguration()
        for sheet in sheets:
            for face in sheet.font_faces:
                config.registry.register_file(
                    str(face.path), family=face.family, weight=face.weight, style=face.style
                )
        # The argument is accepted for source compatibility. Presentational
        # attributes beyond Serpentype's controlled HTML/CSS subset are not emulated.
        _ = presentational_hints
        css_text = "\n".join(sheet.text for sheet in sheets)
        base_dir = _local_path(".", self.base_url, self._inferred_base)
        with TemporaryDirectory(prefix="serpentype-images-") as temp:
            html = _materialize_data_images(
                self._source, Path(temp), self.base_url, self._inferred_base
            )
            prepared = Renderer(fonts=config.registry, base_dir=str(base_dir)).layout(
                html, css_text
            )
        return Document(prepared)

    def write_pdf(self, target=None, *, stylesheets=None, font_config=None,
                  presentational_hints=False):
        return self.render(
            stylesheets=stylesheets,
            font_config=font_config,
            presentational_hints=presentational_hints,
        ).write_pdf(target)
