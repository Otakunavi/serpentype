"""Bounded Python resource loading for the compatibility API."""

from __future__ import annotations

import base64
import io
import math
import mimetypes
import re
from collections import OrderedDict
from dataclasses import dataclass
from enum import Enum
from importlib.resources import files
from pathlib import Path
from urllib.parse import unquote, unquote_to_bytes, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener


_IMG = re.compile(r"<img\b[^>]*>", re.IGNORECASE | re.DOTALL)
_SRC = re.compile(
    r"\bsrc\s*=\s*(?:\"(?P<double>[^\"]*)\"|'(?P<single>[^']*)'|(?P<bare>[^\s>]+))",
    re.IGNORECASE | re.DOTALL,
)
_FONT_FACE = re.compile(r"@font-face\s*\{([^{}]*)\}", re.IGNORECASE | re.DOTALL)
_URL = re.compile(r"url\(\s*(['\"]?)(.*?)\1\s*\)", re.IGNORECASE | re.DOTALL)


class ResourceKind(str, Enum):
    IMAGE = "image"
    FONT = "font"
    CSS = "css"
    OTHER = "other"


@dataclass(frozen=True)
class Resource:
    data: bytes
    url: str
    media_type: str | None = None


class ResourceLimitError(ValueError):
    """A resource count or byte limit was exceeded."""


class ResourceCancelledError(RuntimeError):
    """Resource loading was cancelled."""


class _Redirects(HTTPRedirectHandler):
    def __init__(self, limit, allowed_schemes, allowed_hosts):
        self.limit = limit
        self.allowed_schemes = allowed_schemes
        self.allowed_hosts = allowed_hosts
        self.count = 0

    def redirect_request(self, request, fp, code, msg, headers, newurl):
        self.count += 1
        if self.count > self.limit:
            raise ResourceLimitError("resource redirect limit exceeded")
        parsed = urlsplit(newurl)
        if parsed.scheme not in self.allowed_schemes or (
            self.allowed_hosts is not None and parsed.hostname not in self.allowed_hosts
        ):
            raise ValueError(f"resource redirect is not allowed: {newurl}")
        return super().redirect_request(request, fp, code, msg, headers, newurl)


class ResourceLoader:
    """Load bounded local, data, package, memory, or explicitly allowed HTTP resources."""

    def __init__(self, *, mapping=None, callback=None, base_dir=None,
                 allowed_schemes=("file", "data", "package", "memory"),
                 allowed_hosts=None, timeout=10.0, redirect_limit=3,
                 max_resource_bytes=20_000_000, max_total_bytes=100_000_000,
                 max_resources=1000, cache_bytes=50_000_000, cancel_token=None):
        if not math.isfinite(timeout) or timeout <= 0:
            raise ValueError("timeout must be positive and finite")
        for name, value in (("redirect_limit", redirect_limit),
                            ("max_resource_bytes", max_resource_bytes),
                            ("max_total_bytes", max_total_bytes),
                            ("max_resources", max_resources), ("cache_bytes", cache_bytes)):
            if not isinstance(value, int) or value < 0:
                raise ValueError(f"{name} must be a nonnegative integer")
        self.mapping = dict(mapping or {})
        self.callback = callback
        self.base_dir = Path(base_dir).resolve() if base_dir is not None else None
        self.allowed_schemes = frozenset(allowed_schemes)
        self.allowed_hosts = None if allowed_hosts is None else frozenset(allowed_hosts)
        self.timeout = timeout
        self.redirect_limit = redirect_limit
        self.max_resource_bytes = max_resource_bytes
        self.max_total_bytes = max_total_bytes
        self.max_resources = max_resources
        self.cache_bytes = cache_bytes
        self.cancel_token = cancel_token
        self._cache = OrderedDict()
        self._cache_size = 0
        self._total_bytes = 0
        self._count = 0

    @property
    def cache_size(self) -> int:
        """Number of resource payload bytes currently retained by the LRU cache."""
        return self._cache_size

    @property
    def cached_resources(self) -> int:
        """Number of resource entries currently retained by the LRU cache."""
        return len(self._cache)

    def _check_cancelled(self):
        token = self.cancel_token
        if token is not None and (
            (callable(token) and token()) or getattr(token, "cancelled", False)
        ):
            raise ResourceCancelledError("resource loading cancelled")

    def _read_bounded(self, stream):
        output = io.BytesIO()
        while True:
            self._check_cancelled()
            chunk = stream.read(min(65536, self.max_resource_bytes + 1 - output.tell()))
            if not chunk:
                break
            output.write(chunk)
            if output.tell() > self.max_resource_bytes:
                raise ResourceLimitError("single resource byte limit exceeded")
        return output.getvalue()

    def _coerce(self, value, url):
        if isinstance(value, Resource):
            return value
        if isinstance(value, dict):
            data = value.get("string")
            if data is None and "file_obj" in value:
                data = self._read_bounded(value["file_obj"])
            if data is None and "filename" in value:
                with Path(value["filename"]).open("rb") as stream:
                    data = self._read_bounded(stream)
            if data is None:
                raise ValueError("resource callback must return string, file_obj or filename")
            return Resource(bytes(data), url, value.get("mime_type"))
        return Resource(bytes(value), url)

    def load(self, url: str, kind: ResourceKind) -> Resource:
        self._check_cancelled()
        if not isinstance(kind, ResourceKind):
            kind = ResourceKind(kind)
        key = (url, kind)
        if key in self._cache:
            self._cache.move_to_end(key)
            return self._cache[key]
        if self._count >= self.max_resources:
            raise ResourceLimitError("resource count limit exceeded")
        parsed = urlsplit(url)
        scheme = parsed.scheme or "file"
        if scheme not in self.allowed_schemes:
            raise ValueError(f"resource URL scheme is not allowed: {scheme}")
        if url in self.mapping:
            resource = self._coerce(self.mapping[url], url)
        elif self.callback is not None:
            resource = self._coerce(self.callback(url), url)
        elif scheme == "data":
            header, separator, body = url[5:].partition(",")
            if not separator:
                raise ValueError("invalid data URL")
            media_type = header.split(";", 1)[0] or "text/plain"
            # Percent encoding can use at most three input bytes per decoded byte.
            if len(body) > max(8, self.max_resource_bytes * 4):
                raise ResourceLimitError("single resource byte limit exceeded")
            raw = unquote_to_bytes(body)
            data = base64.b64decode(raw, validate=True) if header.endswith(";base64") else raw
            resource = Resource(data, url, media_type)
        elif scheme == "package":
            if not parsed.netloc or not parsed.path:
                raise ValueError("package URL must include package and path")
            parts = Path(unquote(parsed.path).lstrip("/")).parts
            if ".." in parts:
                raise ValueError("package path escapes its root")
            target = files(parsed.netloc)
            for part in parts:
                target = target.joinpath(part)
            with target.open("rb") as stream:
                resource = Resource(self._read_bounded(stream), url)
        elif scheme == "file":
            if parsed.netloc not in ("", "localhost"):
                raise ValueError("non-local file host is not allowed")
            path = Path(unquote(parsed.path if parsed.scheme else url))
            if not path.is_absolute():
                path = (self.base_dir or Path.cwd()) / path
            path = path.resolve()
            if self.base_dir is not None and not path.is_relative_to(self.base_dir):
                raise ValueError(f"resource is outside base_dir: {url}")
            with path.open("rb") as stream:
                resource = Resource(self._read_bounded(stream), url)
        elif scheme in ("http", "https"):
            if self.allowed_hosts is not None and parsed.hostname not in self.allowed_hosts:
                raise ValueError(f"resource host is not allowed: {parsed.hostname}")
            redirects = _Redirects(self.redirect_limit, self.allowed_schemes, self.allowed_hosts)
            opener = build_opener(redirects)
            with opener.open(Request(url), timeout=self.timeout) as response:
                resource = Resource(self._read_bounded(response), url,
                                    response.headers.get_content_type())
        else:
            raise ValueError(f"resource not found: {url}")
        size = len(resource.data)
        if size > self.max_resource_bytes or self._total_bytes + size > self.max_total_bytes:
            raise ResourceLimitError("resource byte limit exceeded")
        self._total_bytes += size
        self._count += 1
        if size <= self.cache_bytes:
            while self._cache and self._cache_size + size > self.cache_bytes:
                _, old = self._cache.popitem(last=False)
                self._cache_size -= len(old.data)
            self._cache[key] = resource
            self._cache_size += size
        return resource


def _resource_media_type(resource, source, kind):
    if resource.media_type:
        return resource.media_type
    guessed = mimetypes.guess_type(urlsplit(source).path)[0]
    if guessed:
        return guessed
    data = resource.data.lstrip()
    if kind == ResourceKind.IMAGE:
        if data.startswith(b"\x89PNG\r\n\x1a\n"):
            return "image/png"
        if data.startswith(b"\xff\xd8\xff"):
            return "image/jpeg"
        if data.startswith((b"GIF87a", b"GIF89a")):
            return "image/gif"
        if data.startswith(b"RIFF") and data[8:12] == b"WEBP":
            return "image/webp"
        if data.startswith(b"<svg") or b"<svg" in data[:512]:
            return "image/svg+xml"
    return "application/octet-stream"


def _prepare_renderer_sources(html, css, loader, base_dir, cancel_token=None):
    """Resolve direct Renderer resources through one bounded public loader."""
    def resource_url(source):
        if urlsplit(source).scheme or loader.base_dir is not None:
            return source
        return str((Path(base_dir) / unquote(source)).resolve())

    def as_data_uri(source, kind):
        if source.startswith("data:"):
            return source
        try:
            resource = loader.load(resource_url(source), kind)
        except Exception as error:
            # Keep the original source token even when the loader's error
            # only reports a byte/count limit and omits the requested URL.
            error.resource_source = source
            raise
        media_type = _resource_media_type(resource, source, kind)
        encoded = base64.b64encode(resource.data).decode("ascii")
        return f"data:{media_type};base64,{encoded}"

    def rewrite_image(match):
        tag = match.group()
        source_match = _SRC.search(tag)
        if source_match is None:
            return tag
        group = next(
            name for name in ("double", "single", "bare")
            if source_match.group(name) is not None
        )
        source = unquote(source_match.group(group))
        replacement = as_data_uri(source, ResourceKind.IMAGE)
        start, end = source_match.span(group)
        return tag[:start] + replacement + tag[end:]

    def rewrite_font_face(match):
        block = match.group()

        def rewrite_url(url_match):
            source = url_match.group(2)
            replacement = as_data_uri(source, ResourceKind.FONT)
            return f'url("{replacement}")'

        return _URL.sub(rewrite_url, block)

    previous_token = loader.cancel_token
    if cancel_token is not None:
        loader.cancel_token = cancel_token
    try:
        html = _IMG.sub(rewrite_image, html)
        html = _FONT_FACE.sub(rewrite_font_face, html)
        css = _FONT_FACE.sub(rewrite_font_face, css)
        return html, css
    finally:
        loader.cancel_token = previous_token
