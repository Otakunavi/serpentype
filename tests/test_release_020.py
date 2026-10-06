"""Regression cases for capabilities added on the road to 0.2.0."""
import base64
import hashlib
import io
import json
import re
import tempfile
import unittest
from unittest import mock
from urllib.request import HTTPRedirectHandler
from pathlib import Path

import serpentype


class Release020Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        fonts = serpentype.FontRegistry()
        fonts.register_file(serpentype.bundled_font_path(), "Noto Sans")
        fonts.register_file(serpentype.bundled_font_path(700), "Noto Sans", 700)
        cls.renderer = serpentype.Renderer(fonts=fonts)

    def test_capabilities_are_machine_readable(self):
        matrix = serpentype.capabilities()
        self.assertEqual(matrix["diagnostics.filters"], "full")
        self.assertEqual(matrix["diagnostics.source-locations"], "full")
        self.assertEqual(matrix["strict.typed-errors"], "full")
        self.assertEqual(matrix["ffi.panic-guard"], "full")
        self.assertEqual(matrix["pdf.deterministic"], "partial")
        self.assertEqual(matrix["css.text-align.justify"], "partial")
        self.assertTrue(self.renderer.supports("css.text-align.justify"))
        self.assertTrue(self.renderer.supports("pdf.deterministic"))
        self.assertEqual(matrix["image.data-uri"], "partial")
        self.assertEqual(matrix["resource.local-base-dir-confinement"], "full")
        self.assertEqual(matrix["font.shaping"], "full")
        self.assertTrue(self.renderer.supports("font.shaping"))
        self.assertTrue(serpentype.Renderer(experimental_shaping=True).supports("font.shaping"))
        self.assertEqual(matrix["image.svg"], "partial")
        self.assertTrue(self.renderer.supports("image.svg"))
        self.assertEqual(matrix["font.bidi.mixed"], "full")
        self.assertEqual(matrix["font.bidi.controls"], "full")
        self.assertTrue(self.renderer.supports("font.bidi.controls"))
        self.assertTrue(
            serpentype.Renderer(experimental_shaping=True).supports("font.bidi.controls")
        )
        for capability in (
            "css.length.percent-edges", "css.color.alpha",
            "css.overflow-clipping", "css.margin-collapse", "css.border.per-side",
            "css.border.radius", "font.missing-glyph-source",
        ):
            self.assertEqual(matrix[capability], "full")
            self.assertTrue(self.renderer.supports(capability))
        self.assertEqual(matrix["css.opacity"], "partial")
        self.assertTrue(self.renderer.supports("css.opacity"))
        for capability in ("font.cff", "font.woff", "font.woff2",
                           "font.variable-weight", "font.synthetic-bold",
                           "font.synthetic-italic", "font.fallback.emoji-mono",
                           "font.fallback.cluster", "font.variable-width",
                           "font.missing-glyph-diagnostic"):
            self.assertEqual(matrix[capability], "partial")
            self.assertTrue(self.renderer.supports(capability))
        self.assertEqual(matrix["html.table.colspan"], "full")
        self.assertEqual(matrix["html.table.rowspan"], "full")
        self.assertTrue(self.renderer.supports("html.table.rowspan"))
        self.assertEqual(matrix["html.table.repeat-tfoot"], "full")
        self.assertEqual(matrix["resource.loader"], "partial")
        self.assertEqual(matrix["css.position.relative"], "partial")
        self.assertEqual(matrix["css.position.absolute"], "partial")
        self.assertEqual(matrix["css.position.fixed"], "partial")
        self.assertEqual(matrix["css.transform"], "partial")
        self.assertEqual(matrix["layout.flex.fragmentation"], "full")
        self.assertEqual(matrix["layout.grid.fragmentation"], "full")
        self.assertEqual(matrix["css.paged.recto-verso"], "partial")
        self.assertEqual(matrix["css.paged.pseudo-pages"], "partial")
        self.assertEqual(matrix["layout.gap"], "partial")
        self.assertEqual(matrix["css.box-sizing"], "partial")
        self.assertEqual(matrix["css.box.min-max"], "partial")
        self.assertEqual(matrix["css.visibility"], "partial")
        self.assertTrue(self.renderer.supports("css.visibility"))
        self.assertEqual(matrix["css.display.inline-block"], "partial")
        self.assertEqual(matrix["css.hyphens.manual"], "full")
        self.assertEqual(matrix["html.table.border-model"], "full")
        self.assertEqual(matrix["html.table.nested"], "full")
        self.assertEqual(matrix["html.table.sizing"], "full")
        self.assertEqual(matrix["html.table.presentational-hints"], "full")

    def test_diagnostic_filters_callbacks_limits_and_typed_strict_error(self):
        html = "<blink>ignored</blink><p style='text-decoration:overline'>kept</p>"
        seen = []
        renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(),
            diagnostic_deny_codes=["html-tag"],
            diagnostic_allow_severities=["warning"],
            max_diagnostics=1,
            diagnostic_callback=seen.append,
        )
        document = renderer.layout(html)
        self.assertEqual(len(document.diagnostics), 1)
        self.assertEqual(len(seen), 1)
        self.assertEqual(seen[0].severity, "warning")
        self.assertEqual(document.render_stats.diagnostic_count, 1)

        strict_renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(), strict=True, source_name="strict.html"
        )
        with self.assertRaises(serpentype.StrictModeError) as strict_error:
            strict_renderer.layout("<p>ok</p>\n<blink>lost</blink>")
        self.assertEqual(strict_error.exception.source, "strict.html")
        self.assertEqual(
            (strict_error.exception.line, strict_error.exception.column), (2, 1)
        )
        with self.assertRaises(serpentype.StrictModeError) as strict_link:
            strict_renderer.layout("<p>ok</p>\n<a href='#missing'>link</a>")
        self.assertEqual(strict_link.exception.source, "strict.html")
        self.assertEqual(
            (strict_link.exception.line, strict_link.exception.column), (2, 10)
        )
        with self.assertRaises(serpentype.StrictModeError) as strict_inline_css:
            strict_renderer.layout(
                "<p>ok</p>\n<p style='height:nonsense'>lost</p>"
            )
        self.assertEqual(strict_inline_css.exception.source, "strict.html")
        self.assertEqual(
            (strict_inline_css.exception.line, strict_inline_css.exception.column),
            (2, 11),
        )
        with self.assertRaises(serpentype.StrictModeError) as strict_stylesheet:
            strict_renderer.layout("<p>lost</p>", "p { color: red }\n p { height: nonsense }")
        self.assertEqual(strict_stylesheet.exception.source, "<css>")
        self.assertEqual(
            (strict_stylesheet.exception.line, strict_stylesheet.exception.column),
            (2, 6),
        )
        serpentype.Renderer(
            fonts=self.renderer_fonts(), strict=True, diagnostic_deny_codes=["html-tag"]
        ).layout("<blink>lost</blink><p>retained</p>")

    def test_diagnostic_allowlist_and_validation(self):
        diagnostics = serpentype.Renderer(
            fonts=self.renderer_fonts(), diagnostic_allow_codes=["html-tag"]
        ).layout(
            "<blink>x</blink><p style='text-decoration:overline'>y</p>"
        ).diagnostics
        self.assertEqual([item.code for item in diagnostics], ["html-tag"])
        with self.assertRaises(ValueError):
            serpentype.Renderer(
                fonts=self.renderer_fonts(), diagnostic_allow_severities=["fatal"]
            )

    def test_html_and_css_diagnostics_include_source_locations(self):
        renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(), source_name="invoice.html"
        )
        html_diagnostic = renderer.layout("<p>ok</p>\n<blink>x</blink>").diagnostics[0]
        self.assertEqual(html_diagnostic.source, "invoice.html")
        self.assertEqual((html_diagnostic.line, html_diagnostic.column), (2, 1))

        css_diagnostic = renderer.layout(
            "<p>ok</p>", "p { text-decoration: overline; }"
        ).diagnostics[0]
        self.assertEqual(css_diagnostic.source, "<css>")
        self.assertEqual((css_diagnostic.line, css_diagnostic.column), (1, 5))

    def test_resource_and_layout_errors_include_source_locations(self):
        renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(), source_name="broken.html"
        )
        with self.assertRaises(ValueError) as missing_image:
            renderer.layout("<p>ok</p>\n<img src='missing.png'>")
        self.assertEqual(missing_image.exception.source, "broken.html")
        self.assertEqual(
            (missing_image.exception.line, missing_image.exception.column), (2, 1)
        )

        with self.assertRaises(ValueError) as after_script:
            renderer.layout(
                "<!-- comment > <img src='comment-fake.png'> -->\n"
                '<script>const markup = "<img src=\'fake.png\'>";</script>\n'
                "<img src='missing.png'>"
            )
        self.assertEqual(
            (after_script.exception.line, after_script.exception.column), (3, 1)
        )

        loader = serpentype.ResourceLoader(allowed_schemes=("memory",))
        renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(),
            source_name="resource.html",
            resource_loader=loader,
        )
        with self.assertRaises(ValueError) as resource_error:
            renderer.layout("<p>ok</p>\n<img src='memory:missing'>")
        self.assertEqual(resource_error.exception.source, "resource.html")
        self.assertEqual(
            (resource_error.exception.line, resource_error.exception.column), (2, 11)
        )

        limited_loader = serpentype.ResourceLoader(
            mapping={"memory:too-large": b"oversized"}, max_resource_bytes=1
        )
        renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(),
            source_name="limited.html",
            resource_loader=limited_loader,
        )
        with self.assertRaises(serpentype.ResourceLimitError) as limited_error:
            renderer.layout("<p>ok</p>\n<img src='memory:too-large'>")
        self.assertEqual(limited_error.exception.source, "limited.html")
        self.assertEqual(
            (limited_error.exception.line, limited_error.exception.column), (2, 11)
        )

    def test_css_font_resource_errors_include_css_source_location(self):
        with tempfile.TemporaryDirectory() as directory:
            renderer = serpentype.Renderer(
                fonts=self.renderer_fonts(), base_dir=directory,
                source_name="font.css",
            )
            with self.assertRaises(ValueError) as error:
                renderer.layout(
                    "<p>font</p>",
                    "@font-face { font-family: Missing; src: url(missing.ttf) }",
                )
        self.assertEqual(error.exception.source, "<css>")
        self.assertEqual(
            (error.exception.line, error.exception.column),
            (1, "@font-face { font-family: Missing; src: url(missing.ttf) }".index("missing.ttf") + 1),
        )

        css = "@font-face { font-family: Local; src: local('FontName') }"
        with self.assertRaisesRegex(ValueError, "unsupported @font-face src") as bad_src:
            renderer.layout("<p>font</p>", css)
        self.assertEqual(bad_src.exception.source, "<css>")
        self.assertEqual(
            (bad_src.exception.line, bad_src.exception.column),
            (1, css.index("local('FontName')") + 1),
        )

    def test_css_parser_errors_include_the_offending_source_position(self):
        renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(), source_name="malformed.html"
        )
        with self.assertRaises(ValueError) as unclosed_comment:
            renderer.layout("<p>ok</p>", "p { color: red; }\n/* unfinished")
        self.assertEqual(unclosed_comment.exception.source, "<css>")
        self.assertEqual(
            (unclosed_comment.exception.line, unclosed_comment.exception.column), (2, 1)
        )

        with self.assertRaises(ValueError) as unclosed_rule:
            renderer.layout("<p>ok</p>", "p { color: red;")
        self.assertEqual(unclosed_rule.exception.source, "<css>")
        self.assertEqual(
            (unclosed_rule.exception.line, unclosed_rule.exception.column), (1, 1)
        )

        strict_renderer = serpentype.Renderer(
            fonts=self.renderer_fonts(), strict=True
        )
        with self.assertRaises(serpentype.StrictModeError) as strict_css:
            strict_renderer.layout(
                "<p>ok</p>", "p { color: red; }\n@supports (display: grid) { p { color: blue } }"
            )
        self.assertEqual(strict_css.exception.source, "<css>")
        self.assertEqual(
            (strict_css.exception.line, strict_css.exception.column), (2, 1)
        )

    def test_all_parser_and_conversion_diagnostic_kinds_have_locations(self):
        payload = base64.b64encode(
            Path(__file__).parent.joinpath("fixtures", "alpha.png").read_bytes()
        ).decode("ascii")
        html = (
            "<script>not-run</script>\n<MARQUEE>ignored</MARQUEE>"
            "<a href='#missing&amp;x'>link</a>"
            f"<img src='data:image/png;base64,{payload}' style='background:red'>"
            "<span style='position:absolute'>P</span>"
            "<span style='transform:scale(2)'>T</span>"
            "<div style='break-inside:avoid'>D</div>"
            "<span style='break-before:page'>B</span>"
        )
        css = (
            "@page { size: not-a-size; }\n"
            "@supports (display: grid) { p { color: red } }\n"
            "p:hover { color: red }\n"
            "@font-face { font-family: MissingSource; }\n"
            "p { height: not-a-length; text-decoration: overline; }"
        )
        diagnostics = serpentype.Renderer(
            fonts=self.renderer_fonts(), source_name="all-diagnostics.html"
        ).layout(html, css).diagnostics
        self.assertGreaterEqual(len(diagnostics), 10)
        for item in diagnostics:
            with self.subTest(code=item.code, message=item.message):
                self.assertIsNotNone(item.source)
                self.assertGreaterEqual(item.line, 1)
                self.assertGreaterEqual(item.column, 1)

    def test_css_parser_warning_families_keep_original_line_and_column(self):
        css = (
            "@page { size: not-a-size; margin: 1pt 2pt 3pt 4pt 5pt; bleed: -2pt; "
            "unknown-page-property: x }\n"
            "@page:foo:bar { size: A4 }\n"
            "@page { @top-left { font-size: nope; color: nope; strange: x } "
            "@center { content: 'x' } }\n"
            "@supports (display: grid) { p { color: red } }\n"
            "@font-face { font-weight: 400 }\n"
            "p:hover { color: red }\n"
            "p { height: nonsense }\n"
            "trailing-garbage"
        )
        diagnostics = self.renderer.layout("<p>CSS locations</p>", css).diagnostics
        codes = {item.code for item in diagnostics}
        self.assertTrue(
            {"css-value", "css-property", "at-rule", "font-face", "selector", "css-syntax"}
            .issubset(codes),
            codes,
        )
        self.assertGreaterEqual(len(diagnostics), 12)
        for item in diagnostics:
            with self.subTest(code=item.code, message=item.message):
                self.assertEqual(item.source, "<css>")
                self.assertIsNotNone(item.line)
                self.assertIsNotNone(item.column)

    def test_unsupported_declaration_locations_select_the_matching_declaration(self):
        inline = "<p>before</p>\n<span style='text-decoration:overline'>x</span>"
        inline_warning = next(
            diagnostic for diagnostic in self.renderer.layout(inline).diagnostics
            if diagnostic.code == "css-property" and diagnostic.property == "text-decoration"
        )
        self.assertEqual(inline_warning.source, "<html>")
        self.assertEqual(
            (inline_warning.line, inline_warning.column),
            (2, inline.splitlines()[1].index("text-decoration") + 1),
        )

        css = "p { color: red }\nspan { text-decoration: overline }"
        css_warning = next(
            diagnostic for diagnostic in self.renderer.layout("<span>x</span>", css).diagnostics
            if diagnostic.code == "css-property" and diagnostic.property == "text-decoration"
        )
        self.assertEqual(css_warning.source, "<css>")
        self.assertEqual((css_warning.line, css_warning.column), (2, 8))

        html = (
            "<!-- <span style='text-decoration:overline'>comment</span> -->\n"
            "<script>const markup = \"<span style='text-decoration:overline'>\";</script>\n"
            "<span style='text-decoration:overline'>actual</span>"
        )
        html_warning = next(
            diagnostic for diagnostic in self.renderer.layout(html).diagnostics
            if diagnostic.code == "css-property" and diagnostic.property == "text-decoration"
        )
        self.assertEqual((html_warning.line, html_warning.column), (3, 14))

    def test_embedded_css_diagnostic_does_not_match_earlier_html_text(self):
        html = (
            "<p>not-a-size</p>\n<style>\n@page { size: not-a-size; }\n</style>"
            "<p>content</p>"
        )
        warning = next(
            diagnostic for diagnostic in self.renderer.layout(html).diagnostics
            if diagnostic.code == "css-value" and "@page size" in diagnostic.message
        )
        self.assertEqual(warning.source, "<html>")
        self.assertEqual((warning.line, warning.column), (3, 15))

    def test_renderer_existing_positional_constructor_order_is_preserved(self):
        renderer = serpentype.Renderer(
            self.renderer_fonts(), ".", False, None, True, 144.0,
            False, False, False, None,
        )
        self.assertEqual(renderer.layout("<p>Compatible</p>").page_count, 1)

    def test_inline_block_and_manual_soft_hyphen_export(self):
        from pypdf import PdfReader
        html = ("<p style='margin:0;width:70pt'>A"
                "<span style='display:inline-block;width:28pt;padding:2pt;"
                "background:red'>one two</span>Z</p>"
                "<p style='margin:0;width:55pt;overflow-wrap:normal;hyphens:manual'>"
                "encyclo&shy;pedia</p>")
        document = self.renderer.layout(html)
        pdf = bytes(document.to_pdf())
        page = PdfReader(io.BytesIO(pdf)).pages[0]
        extracted = page.extract_text()
        self.assertIn("one", extracted)
        self.assertIn("two", extracted)
        self.assertIn("encyclo-pedia", extracted.replace("\n", ""))
        self.assertIn(b" re", page.get_contents().get_data())

    def test_visibility_hidden_keeps_content_out_of_pdf_text(self):
        from pypdf import PdfReader
        doc = self.renderer.layout(
            "<p>A<span style='visibility:hidden'>SECRET</span>B</p>"
            "<p style='visibility:hidden;background:red'>BLOCK</p>"
            "<p>C</p>"
        )
        extracted = PdfReader(io.BytesIO(bytes(doc.to_pdf()))).pages[0].extract_text()
        self.assertIn("A", extracted)
        self.assertIn("B", extracted)
        self.assertIn("C", extracted)
        self.assertNotIn("SECRET", extracted)
        self.assertNotIn("BLOCK", extracted)

    def test_positioned_fixed_and_transformed_content_exports(self):
        from pypdf import PdfReader
        html = ("<div class='fixed'>F</div>"
                "<div class='host'><a href='https://example.test' class='absolute'>X</a>"
                "<p>A</p></div><p style='break-before:page'>B</p>")
        css = ("@page { size:180pt 120pt; margin:10pt } p { margin:0 } "
               ".fixed { position:fixed; right:3pt; top:2pt; width:20pt; z-index:3 } "
               ".host { position:relative; width:100pt; height:45pt; overflow:hidden } "
               ".absolute { display:block; position:absolute; left:20%; top:8pt; width:30pt; "
               "transform:translate(4pt,2pt) scale(1.1) rotate(5deg) }")
        pdf = bytes(self.renderer.layout(html, css).to_pdf())
        reader = PdfReader(io.BytesIO(pdf))
        self.assertEqual(len(reader.pages), 2)
        for page in reader.pages:
            self.assertIn("F", page.extract_text())
        first_stream = reader.pages[0].get_contents().get_data()
        self.assertIn(b" cm", first_stream)
        self.assertEqual(len(reader.pages[0]["/Annots"]), 1)

    def test_extended_flex_grid_and_fragmentation_export(self):
        from pypdf import PdfReader
        html = ("<div class='flex'><div>A</div><div>B</div><div>C</div></div>"
                "<div class='grid'><div class='span'>S</div><div>D</div><div>E</div></div>")
        css = ("@page { size:180pt 110pt; margin:10pt } "
               ".flex { display:flex; flex-direction:row; flex-wrap:wrap; width:140pt; gap:5pt } "
               ".flex > div { flex:1 1 60pt; height:24pt } "
               ".grid { display:grid; width:140pt; grid-template-columns:40pt 25% minmax(20pt,1fr); "
               "grid-template-rows:repeat(2,30pt); gap:5pt; align-items:center } "
               ".span { grid-column:2 / span 2 }")
        document = self.renderer.layout(html, css)
        self.assertGreaterEqual(document.page_count, 2)
        extracted = "".join(
            page.extract_text() for page in PdfReader(io.BytesIO(bytes(document.to_pdf()))).pages
        )
        for marker in "ABC SDE".replace(" ", ""):
            self.assertIn(marker, extracted)

    def test_resource_loader_mapping_and_limits(self):
        payload = Path(__file__).parent.joinpath("fixtures", "alpha.png").read_bytes()
        loader = serpentype.ResourceLoader(mapping={"memory:logo": payload}, max_resources=1)
        resource = loader.load("memory:logo", serpentype.ResourceKind.IMAGE)
        self.assertEqual(resource.data, payload)
        self.assertEqual(loader.load("memory:logo", serpentype.ResourceKind.IMAGE).data, payload)
        with self.assertRaises(serpentype.ResourceLimitError):
            loader.load("data:text/plain,another", serpentype.ResourceKind.IMAGE)

        aggregate = serpentype.ResourceLoader(
            mapping={"memory:a": b"1234", "memory:b": b"5678"},
            max_total_bytes=7, cache_bytes=1,
        )
        aggregate.load("memory:a", serpentype.ResourceKind.OTHER)
        with self.assertRaises(serpentype.ResourceLimitError):
            aggregate.load("memory:b", serpentype.ResourceKind.OTHER)

    def test_bounded_resource_cache_evicts_old_entries(self):
        loader = serpentype.ResourceLoader(
            mapping={f"memory:{i}": bytes([i]) * 4 for i in range(8)},
            cache_bytes=8,
        )
        for i in range(8):
            loader.load(f"memory:{i}", serpentype.ResourceKind.OTHER)
            self.assertLessEqual(loader.cache_size, 8)
            self.assertLessEqual(loader.cached_resources, 2)
        loader.load("memory:0", serpentype.ResourceKind.OTHER)
        self.assertEqual(loader.cache_size, 8)

    def test_seeded_adversarial_html_css_property_corpus(self):
        import random

        rng = random.Random(0x0200)
        tags = ["p", "div", "span", "table", "td", "h2", "a", "section"]
        properties = ["width", "margin", "padding", "color", "display", "break-before"]
        values = ["0", "1px", "50%", "auto", "page", "rgb(1 2 3 / 20%)", "nonsense"]
        for _ in range(80):
            tag = rng.choice(tags)
            prop = rng.choice(properties)
            value = rng.choice(values)
            html = f"<{tag} id='case-{_}' style='{prop}:{value}'>{rng.randrange(1000)}</{tag}>"
            try:
                document = self.renderer.layout(html)
                pdf = bytes(document.to_pdf())
                self.assertTrue(pdf.startswith(b"%PDF"))
                self.assertLess(len(pdf), 2_000_000)
            except (ValueError, RuntimeError):
                # Malformed or unsupported combinations must fail in a controlled way.
                pass

    def test_resource_loader_package_and_data_urls(self):
        loader = serpentype.ResourceLoader()
        font = loader.load("package://serpentype/assets/NotoSans-Regular.ttf",
                           serpentype.ResourceKind.FONT)
        self.assertTrue(font.data.startswith(b"\x00\x01\x00\x00"))
        data = loader.load("data:text/plain;base64,SGVsbG8=", serpentype.ResourceKind.OTHER)
        self.assertEqual(data.data, b"Hello")
        tiny = serpentype.ResourceLoader(max_resource_bytes=2)
        with self.assertRaises(serpentype.ResourceLimitError):
            tiny.load("data:text/plain;base64,SGVsbG8=", serpentype.ResourceKind.OTHER)

    def test_compat_url_fetcher_loads_image_once(self):
        payload = Path(__file__).parent.joinpath("fixtures", "alpha.png").read_bytes()
        calls = []
        def fetch(url):
            calls.append(url)
            return {"string": payload, "mime_type": "image/png"}
        html = serpentype.HTML(string="<img src='memory:logo'><img src='memory:logo'>",
                               url_fetcher=fetch)
        document = html.render()
        self.assertEqual(calls, ["memory:logo"])
        self.assertEqual(document.pages.__len__(), 1)
        self.assertIn(b"/Subtype /Image", document.write_pdf())

    def test_compat_url_fetcher_resolves_remote_base_url(self):
        payload = Path(__file__).parent.joinpath("fixtures", "alpha.png").read_bytes()
        urls = []
        def fetch(url):
            urls.append(url)
            return {"string": payload, "mime_type": "image/png"}
        serpentype.HTML(string="<img src='logo.png'>",
                        base_url="https://example.org/reports/",
                        url_fetcher=fetch).render()
        self.assertEqual(urls, ["https://example.org/reports/logo.png"])

    def test_compat_url_fetcher_registers_font_bytes(self):
        font = Path(serpentype.bundled_font_path()).read_bytes()
        urls = []
        def fetch(url):
            urls.append(url)
            return {"string": font, "mime_type": "font/ttf"}
        css = serpentype.CSS(
            string="@font-face { font-family: Custom; src: url(font.ttf) } p { font-family: Custom }",
            base_url="https://example.org/fonts/", url_fetcher=fetch,
        )
        pdf = serpentype.HTML(string="<p>Font</p>").write_pdf(stylesheets=[css])
        self.assertEqual(urls, ["https://example.org/fonts/font.ttf"])
        self.assertIn(b"/Font", pdf)

    def test_resource_loader_rejects_disallowed_scheme_and_escape(self):
        with tempfile.TemporaryDirectory() as directory:
            loader = serpentype.ResourceLoader(base_dir=directory)
            with self.assertRaises(ValueError):
                loader.load("https://example.org/image.png", serpentype.ResourceKind.IMAGE)
            with self.assertRaises(ValueError):
                loader.load("../outside.png", serpentype.ResourceKind.IMAGE)
        token = serpentype.CancelToken()
        token.cancel()
        loader = serpentype.ResourceLoader(cancel_token=token)
        with self.assertRaises(serpentype.ResourceCancelledError):
            loader.load("data:text/plain,hello", serpentype.ResourceKind.OTHER)

    def test_resource_redirect_limit_counts_the_whole_chain(self):
        from serpentype.resources import _Redirects

        redirects = _Redirects(1, {"https"}, {"example.org"})
        with mock.patch.object(HTTPRedirectHandler, "redirect_request", return_value=None):
            redirects.redirect_request(None, None, 302, "", {},
                                       "https://example.org/second")
            with self.assertRaises(serpentype.ResourceLimitError):
                redirects.redirect_request(None, None, 302, "", {},
                                           "https://example.org/third")

    def test_resource_cancellation_is_polled_between_stream_chunks(self):
        token = serpentype.CancelToken()

        class Stream:
            def __init__(self):
                self.reads = 0

            def read(self, _size):
                self.reads += 1
                if self.reads == 1:
                    token.cancel()
                    return b"chunk"
                return b""

        loader = serpentype.ResourceLoader(
            callback=lambda _url: {"file_obj": Stream()},
            allowed_schemes=("memory",), cancel_token=token,
        )
        with self.assertRaises(serpentype.ResourceCancelledError):
            loader.load("memory:stream", serpentype.ResourceKind.OTHER)

    def test_direct_renderer_uses_resource_loader_for_images_svg_and_fonts(self):
        fixtures = Path(__file__).parent / "fixtures"
        png = (fixtures / "alpha.png").read_bytes()
        font = Path(serpentype.bundled_font_path()).read_bytes()
        svg = (b"<svg xmlns='http://www.w3.org/2000/svg' width='20' height='10'>"
               b"<rect width='20' height='10' fill='red'/></svg>")
        loader = serpentype.ResourceLoader(mapping={
            "memory:logo": serpentype.Resource(png, "memory:logo", "image/png"),
            "memory:shape": serpentype.Resource(svg, "memory:shape", "image/svg+xml"),
            "memory:font": serpentype.Resource(font, "memory:font", "font/ttf"),
        })
        renderer = serpentype.Renderer(fonts=self.renderer_fonts(), resource_loader=loader)
        css = ("@font-face { font-family:Loaded; src:url('memory:font') } "
               "p { font-family:Loaded }")
        pdf = bytes(renderer.layout(
            "<p>Loaded</p><img src='memory:logo'><img src='memory:shape'>", css
        ).to_pdf())
        self.assertIn(b"/Subtype /Image", pdf)
        self.assertIn(b"/Subtype /Form", pdf)
        self.assertIn(b"/Font", pdf)

    def test_renderer_cancel_token_reaches_resource_loader(self):
        png = (Path(__file__).parent / "fixtures" / "alpha.png").read_bytes()
        token = serpentype.CancelToken()
        loader = serpentype.ResourceLoader(
            mapping={"memory:logo": serpentype.Resource(
                png, "memory:logo", "image/png")}
        )
        renderer = serpentype.Renderer(resource_loader=loader)
        token.cancel()
        with self.assertRaises(serpentype.ResourceCancelledError):
            renderer.layout("<img src='memory:logo'>", cancel_token=token)

    def test_diagnostics_are_deduplicated_and_serializable(self):
        doc = self.renderer.layout("<p style='float:left; cursor:pointer'>A</p>"
                                   "<p style='float:left; cursor:pointer'>B</p>")
        diagnostics = doc.diagnostics
        floats = [d for d in diagnostics if d.property == "float"]
        self.assertEqual(len(floats), 1)
        self.assertEqual(floats[0].occurrences, 2)
        self.assertEqual(floats[0].severity, "warning")
        self.assertEqual(json.loads(floats[0].to_json())["property"], "float")
        self.assertFalse(any(d.property == "cursor" for d in diagnostics))

    def test_inline_tags_keep_text_and_styles(self):
        html = ("<p>A <strong>B</strong><em>C</em><u>D</u>"
                "<a href='https://example.org'>E</a><sup>2</sup><sub>3</sub></p>")
        doc = self.renderer.layout(html)
        self.assertEqual(doc.page_count, 1)
        pdf = bytes(doc.to_pdf())
        self.assertIn(b"/Subtype /Type0", pdf)
        self.assertIn(b"/URI", pdf)

    def test_colspan_and_repeating_table_header_preserve_rows(self):
        from pypdf import PdfReader

        rows = "".join(f"<tr><td>Row {index:02d}</td><td>Value {index:02d}</td></tr>"
                       for index in range(24))
        html = ("<table><thead><tr><th colspan='2'>REGISTER</th></tr></thead>"
                f"<tbody>{rows}</tbody></table>")
        css = ("@page { size: 200pt 180pt; margin: 10pt } "
               "table { width: 180pt; table-layout: fixed } "
               "td, th { border: 0.5pt solid black; padding: 1pt; font-size: 9pt; line-height: 11pt }")
        doc = self.renderer.layout(html, css)
        self.assertGreater(doc.page_count, 1)
        pages = PdfReader(io.BytesIO(bytes(doc.to_pdf()))).pages
        texts = [page.extract_text() for page in pages]
        self.assertTrue(all("REGISTER" in text for text in texts))
        combined = "\n".join(texts)
        for index in range(24):
            self.assertEqual(combined.count(f"Row {index:02d}"), 1)
            self.assertEqual(combined.count(f"Value {index:02d}"), 1)

    def test_rowspan_and_repeating_table_footer_preserve_content(self):
        from pypdf import PdfReader

        rows = ("<tr><td rowspan='2'>GROUP</td><td>Row 00</td></tr>"
                "<tr><td>Row 01</td></tr>" +
                "".join(f"<tr><td>Key {index:02d}</td><td>Row {index:02d}</td></tr>"
                         for index in range(2, 18)))
        html = ("<table><thead><tr><th colspan='2'>HEAD</th></tr></thead>"
                f"<tbody>{rows}</tbody>"
                "<tfoot><tr><td colspan='2'>FOOT</td></tr></tfoot></table>")
        css = ("@page { size: 180pt 120pt; margin: 10pt } "
               "table { width: 160pt; table-layout: fixed } "
               "td, th { border: 0.5pt solid black; padding: 1pt; "
               "font-size: 8pt; line-height: 10pt }")
        document = self.renderer.layout(html, css)
        pages = PdfReader(io.BytesIO(bytes(document.to_pdf()))).pages
        self.assertGreater(len(pages), 1)
        texts = [page.extract_text() for page in pages]
        self.assertTrue(all("HEAD" in text for text in texts))
        self.assertTrue(all("FOOT" in text for text in texts))
        combined = "\n".join(texts)
        self.assertEqual(combined.count("GROUP"), 1)
        for index in range(18):
            self.assertEqual(combined.count(f"Row {index:02d}"), 1)

    def test_fragmented_rowspan_and_nested_table_preserve_pdf_text(self):
        from pypdf import PdfReader

        tokens = " ".join(f"TOKEN{index:02d}" for index in range(50))
        html = (
            "<table class='outer'><tbody><tr><td rowspan='2'>" + tokens + "</td>"
            "<td><table class='inner'><tr><td>INNER-A</td></tr><tr><td>INNER-B</td></tr>"
            "</table></td></tr><tr><td>TAIL</td></tr></tbody></table>"
        )
        css = (
            "@page { size:160pt 100pt; margin:10pt } "
            ".outer { width:140pt; table-layout:fixed; border-collapse:collapse } "
            ".inner { width:100%; table-layout:fixed; border-collapse:collapse } "
            "td { border:1pt solid black; padding:1pt; font-size:8pt; line-height:10pt }"
        )
        document = self.renderer.layout(html, css)
        pages = PdfReader(io.BytesIO(bytes(document.to_pdf()))).pages
        self.assertGreater(len(pages), 1)
        text = "\n".join(page.extract_text() for page in pages)
        for index in range(50):
            self.assertEqual(text.count(f"TOKEN{index:02d}"), 1)
        for expected in ("INNER-A", "INNER-B", "TAIL"):
            self.assertEqual(text.count(expected), 1)

    def test_recto_break_exports_a_blank_pdf_page(self):
        from pypdf import PdfReader
        doc = self.renderer.layout("<p>A</p><p style='break-before:right'>B</p>")
        pdf = PdfReader(io.BytesIO(bytes(doc.to_pdf())))
        self.assertEqual(len(pdf.pages), 3)
        self.assertIn("A", pdf.pages[0].extract_text())
        self.assertEqual(pdf.pages[1].extract_text().strip(), "")
        self.assertIn("B", pdf.pages[2].extract_text())

    def test_blank_page_selector_exports_page_size_and_margin_text(self):
        from pypdf import PdfReader
        css = ("@page { size:200pt 100pt; margin:15pt } "
               "@page :right { size:120pt 100pt } "
               "@page :blank { size:160pt 100pt; @top-center { content:'BLANK' } }")
        doc = self.renderer.layout("<p>A</p><p style='break-before:right'>B</p>", css)
        pdf = PdfReader(io.BytesIO(bytes(doc.to_pdf())))
        self.assertEqual([float(page.mediabox.width) for page in pdf.pages],
                         [120.0, 160.0, 120.0])
        self.assertEqual(pdf.pages[1].extract_text().strip(), "BLANK")

    def test_mixed_inline_and_whitespace(self):
        doc = self.renderer.layout("<p>one&nbsp;two <span style='font-size:20pt;color:rgb(255,0,0)'>BIG</span><br>next</p>")
        self.assertFalse(doc.diagnostics)
        self.assertEqual(doc.page_count, 1)
        self.assertEqual(doc.to_pdf(), doc.to_pdf())

    def test_production_shaping_uses_ligatures(self):
        from pypdf import PdfReader

        fonts = self.renderer_fonts()
        shaped = serpentype.Renderer(fonts=fonts).layout("<p>office</p>")
        compatibility = serpentype.Renderer(fonts=fonts, experimental_shaping=False).layout("<p>office</p>")
        self.assertEqual(shaped.render_stats.glyph_count, compatibility.render_stats.glyph_count)
        self.assertLess(shaped.render_stats.glyph_count, len("office"))
        self.assertGreater(shaped.render_stats.shaping_ms, 0)
        self.assertEqual(shaped.to_pdf(), shaped.to_pdf())
        self.assertEqual(PdfReader(io.BytesIO(bytes(shaped.to_pdf()))).pages[0].extract_text(), "office")

    def test_cff_woff_and_woff2_extract_text(self):
        from pypdf import PdfReader

        fixtures = Path(__file__).parent / "fixtures"
        for suffix in ("otf", "otf.woff", "otf.woff2"):
            with self.subTest(suffix=suffix):
                fonts = serpentype.FontRegistry()
                fonts.register_file(str(fixtures / f"SourceSans3-Regular.{suffix}"),
                                    "Source Sans 3")
                pdf = bytes(serpentype.Renderer(fonts=fonts).layout(
                    "<p style='font-family:Source Sans 3'>CFF Àé Привет</p>"
                ).to_pdf())
                self.assertIn(b"/CIDFontType0", pdf)
                self.assertEqual(PdfReader(io.BytesIO(pdf)).pages[0].extract_text(),
                                 "CFF Àé Привет")

    def test_variable_font_weight_changes_pdf_bytes(self):
        from pypdf import PdfReader

        fonts = serpentype.FontRegistry()
        fonts.register_file(str(Path(__file__).parent / "fixtures" /
                                "SourceSans3VF-Upright.ttf"), "Source Sans VF")
        renderer = serpentype.Renderer(fonts=fonts)
        normal = bytes(renderer.layout(
            "<p style='font-family:Source Sans VF'>Variable</p>").to_pdf())
        bold = bytes(renderer.layout(
            "<p style='font-family:Source Sans VF;font-weight:700'>Variable</p>").to_pdf())
        self.assertNotEqual(normal, bold)
        self.assertEqual(PdfReader(io.BytesIO(bold)).pages[0].extract_text(), "Variable")

    def test_variable_font_stretch_changes_pdf_bytes(self):
        from pypdf import PdfReader

        fonts = serpentype.FontRegistry()
        fonts.register_file(str(Path(__file__).parent / "fixtures" / "RobotoFlex-VF.ttf"),
                            "Roboto Flex")
        renderer = serpentype.Renderer(fonts=fonts)
        condensed = bytes(renderer.layout(
            "<p style='font-family:Roboto Flex;font-stretch:75%'>Stretch</p>").to_pdf())
        expanded = bytes(renderer.layout(
            "<p style='font-family:Roboto Flex;font-stretch:125%'>Stretch</p>").to_pdf())
        self.assertNotEqual(condensed, expanded)
        self.assertEqual(PdfReader(io.BytesIO(expanded)).pages[0].extract_text(), "Stretch")

    def test_monochrome_emoji_fallback_extracts_text(self):
        from pypdf import PdfReader

        fonts = self.renderer_fonts()
        fonts.register_file(str(Path(__file__).parent / "fixtures" / "NotoEmoji-VF.ttf"),
                            "Noto Emoji")
        pdf = bytes(serpentype.Renderer(fonts=fonts).layout("<p>Hello 😀</p>").to_pdf())
        self.assertEqual(PdfReader(io.BytesIO(pdf)).pages[0].extract_text(), "Hello 😀")

    def test_missing_glyph_has_structured_exception(self):
        fonts = self.renderer_fonts()
        with self.assertRaises(serpentype.MissingGlyphError) as caught:
            serpentype.Renderer(fonts=fonts).layout(
                "<div>\n<p style='font-family:Noto Sans'>\U00010fff</p></div>")
        error = caught.exception
        self.assertIsInstance(error, ValueError)
        self.assertEqual(error.code_point, 0x10FFF)
        self.assertEqual(error.character, "\U00010fff")
        self.assertEqual(error.requested_family, "Noto Sans")
        self.assertEqual(error.weight, 400)
        self.assertEqual(error.style, "normal")
        self.assertIsNone(error.selected_fallback)
        self.assertEqual(error.source, "<html>")
        self.assertEqual(error.line, 2)
        self.assertGreater(error.column, 1)

    def test_missing_glyph_character_reference_reports_named_source(self):
        fonts = self.renderer_fonts()
        with self.assertRaises(serpentype.MissingGlyphError) as caught:
            serpentype.Renderer(fonts=fonts, source_name="templates/invoice sample.html").layout(
                "<div>ok</div>\n<p>&#x10fff;</p>")
        error = caught.exception
        self.assertEqual(error.source, "templates/invoice sample.html")
        self.assertEqual(error.line, 2)
        self.assertEqual(error.column, 4)

    def test_missing_margin_box_glyph_reports_css_source_location(self):
        css = '@page { size:200pt 100pt; margin:10pt;\n @top-center { content: "\U00010fff" } }'
        with self.assertRaises(serpentype.MissingGlyphError) as caught:
            serpentype.Renderer(fonts=self.renderer_fonts(), source_name="invoice.css").layout(
                "<p>ordinary text</p>", css
            )
        self.assertEqual(caught.exception.source, "<css>")
        self.assertEqual(caught.exception.line, 2)
        self.assertEqual(caught.exception.column, css.splitlines()[1].index("\U00010fff") + 1)

    def test_compat_html_filename_is_used_for_missing_glyph_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invoice source.html"
            source.write_text("<p>&#x10fff;</p>", encoding="utf-8")
            with self.assertRaises(serpentype.MissingGlyphError) as caught:
                serpentype.HTML(filename=source).render()
            self.assertEqual(caught.exception.source, str(source))
            self.assertEqual((caught.exception.line, caught.exception.column), (1, 4))

    def test_compat_html_render_and_write_pdf_accept_render_limits(self):
        html = serpentype.HTML(string=(
            "<p>first</p><p style='break-before:page'>second</p>"
        ))
        limits = serpentype.RenderLimits(max_pages=1)
        with self.assertRaises(serpentype.RenderLimitError):
            html.render(limits=limits)
        with self.assertRaises(serpentype.RenderLimitError):
            html.write_pdf(limits=limits)

    def test_compat_css_woff2_font_face(self):
        font = Path(__file__).parent / "fixtures" / "SourceSans3-Regular.otf.woff2"
        sheet = serpentype.CSS(string=f"@font-face {{ font-family: WebFont; src: url('{font}') }} "
                                     "p { font-family: WebFont }")
        pdf = serpentype.HTML(string="<p>Web Font</p>").write_pdf(stylesheets=[sheet])
        self.assertIn(b"/FontFile3", pdf)

    def test_experimental_arabic_bidi_extracts_logical_text(self):
        from pypdf import PdfReader

        fonts = self.renderer_fonts()
        fonts.register_file(str(Path(__file__).parent / "fixtures" / "NotoSansArabic-Regular.ttf"),
                            "Noto Sans Arabic")
        renderer = serpentype.Renderer(fonts=fonts, experimental_shaping=True)
        doc = renderer.layout("<p style=\"font-family:'Noto Sans'\">مرحبا بالعالم</p>")
        self.assertEqual(doc.page_count, 1)
        self.assertEqual(PdfReader(io.BytesIO(bytes(doc.to_pdf()))).pages[0].extract_text(),
                         "مرحبا بالعالم")

    def test_experimental_mixed_direction_visual_order(self):
        from pypdf import PdfReader

        fonts = self.renderer_fonts()
        fonts.register_file(str(Path(__file__).parent / "fixtures" / "NotoSansArabic-Regular.ttf"),
                            "Noto Sans Arabic")
        renderer = serpentype.Renderer(fonts=fonts, experimental_shaping=True)
        expected = "ABC مرحبا XYZ"
        doc = renderer.layout(f"<p>{expected}</p>")
        pdf = bytes(doc.to_pdf())
        page = PdfReader(io.BytesIO(pdf)).pages[0]
        contents = page.get_contents().get_data()
        actual = re.search(rb"/ActualText <([0-9A-F]+)>", contents)
        self.assertIsNotNone(actual)
        self.assertEqual(bytes.fromhex(actual.group(1).decode()).decode("utf-16"), expected)
        self.assertEqual(page.extract_text(extraction_mode="layout").strip(), "ABC ابحرم XYZ")
        extracted = page.extract_text()
        for word in ("ABC", "مرحبا", "XYZ"):
            self.assertEqual(extracted.count(word), 1)

    def test_experimental_bidi_isolates_are_not_emitted_as_glyphs(self):
        from pypdf import PdfReader

        fonts = self.renderer_fonts()
        fonts.register_file(str(Path(__file__).parent / "fixtures" / "NotoSansArabic-Regular.ttf"),
                            "Noto Sans Arabic")
        renderer = serpentype.Renderer(fonts=fonts, experimental_shaping=True)
        visible = "LTR مرحبا 123 END"
        pdf = bytes(renderer.layout("<p>LTR \u2067مرحبا 123\u2069 END</p>").to_pdf())
        contents = PdfReader(io.BytesIO(pdf)).pages[0].get_contents().get_data()
        actual = re.search(rb"/ActualText <([0-9A-F]+)>", contents)
        self.assertIsNotNone(actual)
        self.assertEqual(bytes.fromhex(actual.group(1).decode()).decode("utf-16"), visible)

    def test_experimental_mixed_hebrew_numbers_and_punctuation(self):
        from pypdf import PdfReader

        fonts = self.renderer_fonts()
        fonts.register_file(str(Path(__file__).parent / "fixtures" / "NotoSansHebrew-VF.ttf"),
                            "Noto Sans Hebrew")
        renderer = serpentype.Renderer(fonts=fonts, experimental_shaping=True)
        expected = "Report (שלום 123) end"
        pdf = bytes(renderer.layout(f"<p>{expected}</p>").to_pdf())
        page = PdfReader(io.BytesIO(pdf)).pages[0]
        contents = page.get_contents().get_data()
        actual = re.search(rb"/ActualText <([0-9A-F]+)>", contents)
        self.assertIsNotNone(actual)
        self.assertEqual(bytes.fromhex(actual.group(1).decode()).decode("utf-16"), expected)
        extracted = page.extract_text()
        for token in ("Report", "123", "end"):
            self.assertEqual(extracted.count(token), 1)
        self.assertEqual(page.extract_text(extraction_mode="layout").count("םולש"), 1)

    def test_experimental_bidi_across_inline_styles(self):
        from pypdf import PdfReader

        fonts = self.renderer_fonts()
        fonts.register_file(str(Path(__file__).parent / "fixtures" / "NotoSansArabic-Regular.ttf"),
                            "Noto Sans Arabic")
        renderer = serpentype.Renderer(fonts=fonts, experimental_shaping=True)
        html = "<p>ABC <span style='color:red'>مرحبا</span> XYZ</p>"
        page = PdfReader(io.BytesIO(bytes(renderer.layout(html).to_pdf()))).pages[0]
        self.assertEqual(page.extract_text(extraction_mode="layout").strip(), "ABC ابحرم XYZ")

    def test_experimental_combining_marks_survive_extraction(self):
        from pypdf import PdfReader

        renderer = serpentype.Renderer(fonts=self.renderer_fonts(), experimental_shaping=True)
        text = "e\u0301 cafe\u0301"
        doc = renderer.layout(f"<p>{text}</p>")
        self.assertEqual(PdfReader(io.BytesIO(bytes(doc.to_pdf()))).pages[0].extract_text(), text)

    def test_deterministic_layout_runs(self):
        html = "<p>Repeated export 123 Привет</p>"
        outputs = [bytes(self.renderer.layout(html).to_pdf()) for _ in range(4)]
        self.assertTrue(all(data == outputs[0] for data in outputs))

    def test_experimental_shaping_is_deterministic(self):
        renderer = serpentype.Renderer(fonts=self.renderer_fonts(), experimental_shaping=True)
        html = "<p>office affinity AV</p>"
        outputs = [bytes(renderer.layout(html).to_pdf()) for _ in range(4)]
        self.assertTrue(all(data == outputs[0] for data in outputs))

    def test_limits_and_cancellation(self):
        limits = serpentype.RenderLimits(max_input_bytes=10)
        renderer = serpentype.Renderer(limits=limits)
        with self.assertRaises(serpentype.RenderLimitError):
            renderer.layout("<p>document too large</p>")
        token = serpentype.CancelToken()
        token.cancel()
        with self.assertRaises(serpentype.RenderCancelledError):
            self.renderer.layout("<p>cancel</p>", cancel_token=token)
        doc = self.renderer.layout("<p>cancel export</p>")
        with self.assertRaises(serpentype.RenderCancelledError):
            doc.to_pdf(cancel_token=token)

    def test_restrict_base_dir_blocks_parent_resource_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            allowed = root / "allowed"
            allowed.mkdir()
            outside = root / "outside.png"
            outside.write_bytes((Path(__file__).parent / "fixtures" / "alpha.png").read_bytes())
            renderer = serpentype.Renderer(
                base_dir=str(allowed),
                limits=serpentype.RenderLimits(restrict_base_dir=True),
            )
            with self.assertRaisesRegex(ValueError, "base_dir"):
                renderer.layout("<img src='../outside.png'>")
            (allowed / "linked.png").symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "base_dir"):
                renderer.layout("<img src='linked.png'>")
            font_outside = root / "outside.ttf"
            font_outside.write_bytes(Path(serpentype.bundled_font_path()).read_bytes())
            (allowed / "linked.ttf").symlink_to(font_outside)
            css = "@font-face {font-family:Local;src:url('linked.ttf')} p {font-family:Local}"
            with self.assertRaisesRegex(ValueError, "base_dir"):
                renderer.layout("<p>Blocked font</p>", css)

    def test_webp_and_gif_first_frame(self):
        fixtures = Path(__file__).parent / "fixtures"
        renderer = serpentype.Renderer(fonts=self.renderer_fonts(), base_dir=str(fixtures))
        pdf = renderer.layout("<img src='blue.webp'><img src='blue.gif'>").to_pdf()
        self.assertEqual(pdf.count(b"/Subtype /Image"), 2)

    def test_data_uri_raster_image(self):
        png = (Path(__file__).parent / "fixtures" / "alpha.png").read_bytes()
        uri = "data:image/png;base64," + base64.b64encode(png).decode("ascii")
        pdf = bytes(self.renderer.layout(f"<img src='{uri}'>").to_pdf())
        self.assertIn(b"/Subtype /Image", pdf)
        self.assertIn(b"/SMask", pdf)
        limited = serpentype.Renderer(limits=serpentype.RenderLimits(max_resource_bytes=len(png) - 1))
        with self.assertRaises(serpentype.RenderLimitError):
            limited.layout(f"<img src='{uri}'>")

    def test_svg_local_data_and_invalid_input(self):
        svg = ("<svg xmlns='http://www.w3.org/2000/svg' width='120' height='80' "
               "viewBox='0 0 120 80'><defs><linearGradient id='g'><stop stop-color='red'/>"
               "<stop offset='1' stop-color='blue'/></linearGradient></defs>"
               "<g transform='translate(5 5)'><rect width='110' height='70' fill='url(#g)'/>"
               "<circle cx='55' cy='35' r='20' fill='white'/></g></svg>")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "shape.svg").write_text(svg)
            renderer = serpentype.Renderer(base_dir=str(root))
            local = bytes(renderer.layout("<img src='shape.svg'>").to_pdf())
            data_uri = "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()
            inline = bytes(renderer.layout(f"<img src='{data_uri}'>").to_pdf())
            self.assertIn(b"/Subtype /Form", local)
            self.assertNotIn(b"/Subtype /Image", local)
            self.assertEqual(local, inline)
            from pypdf import PdfReader
            self.assertEqual(len(PdfReader(io.BytesIO(local)).pages), 1)
            low_dpi = serpentype.Renderer(base_dir=str(root), svg_dpi=96)
            self.assertIn(b"/Subtype /Form",
                          bytes(low_dpi.layout("<img src='shape.svg'>").to_pdf()))
            (root / "invalid.svg").write_text("<svg><path")
            with self.assertRaisesRegex(ValueError, "SVG"):
                renderer.layout("<img src='invalid.svg'>")
            limited = serpentype.Renderer(base_dir=str(root),
                limits=serpentype.RenderLimits(max_image_pixels=100))
            with self.assertRaises(serpentype.RenderLimitError):
                limited.layout("<img src='shape.svg'>")
            with self.assertRaisesRegex(ValueError, "svg_dpi"):
                serpentype.Renderer(svg_dpi=0)

    def test_svg_external_image_is_reported(self):
        svg = ("<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'>"
               "<image href='missing.png' width='10' height='10'/></svg>")
        uri = "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()
        with self.assertRaisesRegex(ValueError, "SVG external image resource"):
            self.renderer.layout(f"<img src='{uri}'>")
        foreign = ("<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'>"
                   "<foreignObject width='10' height='10'><p>Lost</p></foreignObject></svg>")
        foreign_uri = "data:image/svg+xml;base64," + base64.b64encode(foreign.encode()).decode()
        with self.assertRaisesRegex(ValueError, "SVG element is unsupported: foreignObject"):
            self.renderer.layout(f"<img src='{foreign_uri}'>")

    def test_svg_text_uses_registered_fonts(self):
        prefix = "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='30'>"
        text_svg = prefix + "<text x='1' y='22' font-family='Noto Sans'>Hello</text></svg>"
        empty_svg = prefix + "</svg>"
        uri = lambda svg: "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()
        with self.assertRaisesRegex(ValueError, "SVG text requires a registered font"):
            serpentype.Renderer().layout(f"<img src='{uri(text_svg)}'>")
        with_text = bytes(self.renderer.layout(f"<img src='{uri(text_svg)}'>").to_pdf())
        without_text = bytes(self.renderer.layout(f"<img src='{uri(empty_svg)}'>").to_pdf())
        self.assertNotEqual(with_text, without_text)
        missing = prefix + "<text x='1' y='22'>\u0378</text></svg>"
        with self.assertRaisesRegex(ValueError, "SVG text has no registered glyph U\\+0378"):
            self.renderer.layout(f"<img src='{uri(missing)}'>")

    def test_html_compatibility_uses_general_svg_renderer(self):
        svg = ("<svg xmlns='http://www.w3.org/2000/svg' width='30' height='20'>"
               "<circle cx='15' cy='10' r='8' fill='red'/></svg>")
        uri = "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()
        pdf = serpentype.HTML(string=f"<img src='{uri}'>").write_pdf()
        self.assertIn(b"/Subtype /Form", pdf)

    def test_svg_embedded_raster_data_image(self):
        png = (Path(__file__).parent / "fixtures" / "alpha.png").read_bytes()
        png_uri = "data:image/png;base64," + base64.b64encode(png).decode()
        svg = ("<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>"
               f"<image href='{png_uri}' width='20' height='20'/></svg>")
        svg_uri = "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()
        pdf = bytes(self.renderer.layout(f"<img src='{svg_uri}'>").to_pdf())
        self.assertIn(b"/Subtype /Form", pdf)
        self.assertIn(b"/Subtype /Image", pdf)

    def test_alpha_image_and_jpeg_passthrough(self):
        fixtures = Path(__file__).parent / "fixtures"
        renderer = serpentype.Renderer(base_dir=str(fixtures))
        png = bytes(renderer.layout("<img src='alpha.png'>").to_pdf())
        self.assertIn(b"/SMask", png)
        jpeg_source = (fixtures / "blue.jpg").read_bytes()
        jpeg = bytes(renderer.layout("<img src='blue.jpg'>").to_pdf())
        self.assertIn(b"/DCTDecode", jpeg)
        self.assertIn(jpeg_source, jpeg)

    def test_jpeg_exif_orientation(self):
        fixtures = Path(__file__).parent / "fixtures"
        renderer = serpentype.Renderer(base_dir=str(fixtures))
        pdf = bytes(renderer.layout("<img src='rotated-exif.jpg'>").to_pdf())
        self.assertIn(b"/Width 2 /Height 3", pdf)
        self.assertNotIn(b"/DCTDecode", pdf)

    def test_grayscale_jpeg_uses_correct_pdf_colorspace(self):
        fixtures = Path(__file__).parent / "fixtures"
        renderer = serpentype.Renderer(base_dir=str(fixtures))
        pdf = bytes(renderer.layout("<img src='grayscale.jpg'>").to_pdf())
        self.assertIn(b"/ColorSpace /DeviceRGB", pdf)
        self.assertNotIn(b"/DCTDecode", pdf)

    def test_render_stats_and_content_status(self):
        doc = self.renderer.layout("<p>Metrics</p>")
        self.assertEqual(doc.render_stats.page_count, 1)
        self.assertGreater(doc.render_stats.dom_node_count, 0)
        self.assertEqual(doc.overflow_count, 0)
        self.assertEqual(doc.missing_glyphs, [])
        self.assertEqual(doc.unsupported_features, [])
        data = doc.to_pdf()
        self.assertEqual(doc.render_stats.output_size, len(data))

    def test_unsupported_internal_anchor_is_diagnostic(self):
        doc = self.renderer.layout("<p><a href='#missing'>Jump</a></p>")
        self.assertTrue(any(d.code == "pdf-link" for d in doc.diagnostics))
        self.assertNotIn(b"/URI", bytes(doc.to_pdf()))

    def test_internal_link_has_named_destination(self):
        doc = self.renderer.layout("<p><a href='#details'>Jump</a></p>"
                                   "<p id='details' style='break-before:page'>Details</p>")
        self.assertFalse(any(d.code == "pdf-link" for d in doc.diagnostics))
        self.assertEqual(doc.page_count, 2)
        pdf = bytes(doc.to_pdf())
        self.assertIn(b"/Subtype /Link", pdf)
        self.assertIn(b"/Dest [", pdf)
        self.assertIn(b"/Names << /Dests", pdf)

    def test_inline_and_general_id_anchors_export_as_named_destinations(self):
        from pypdf import PdfReader

        html = ("<p><a href='#inline'>Jump</a> to <span id='inline'>target</span>. "
                "<a href='#legacy'>Legacy</a><a name='legacy'>anchor</a></p>")
        pdf = bytes(self.renderer.layout(html).to_pdf())
        reader = PdfReader(io.BytesIO(pdf))
        names = reader.trailer["/Root"]["/Names"]["/Dests"]["/Names"]
        destinations = {str(names[index]) for index in range(0, len(names), 2)}
        self.assertTrue({"inline", "legacy"}.issubset(destinations))
        self.assertTrue(any(annotation.get_object()["/Subtype"] == "/Link"
                            for page in reader.pages
                            for annotation in page.get("/Annots", [])))

    def test_heading_bookmarks_preserve_nested_levels(self):
        from pypdf import PdfReader

        pdf = bytes(self.renderer.layout(
            "<h1>Chapter</h1><h2>Section</h2><h1>Appendix</h1>").to_pdf())
        root = PdfReader(io.BytesIO(pdf)).trailer["/Root"]
        outline = root["/Outlines"].get_object()
        first = outline["/First"].get_object()
        child = first["/First"].get_object()
        self.assertEqual(str(first["/Title"]), "Chapter")
        self.assertEqual(str(child["/Title"]), "Section")
        self.assertIn("/UseOutlines", str(root))

    def test_pdf_attachments_and_configurable_utc_dates(self):
        from pypdf import PdfReader

        pdf = bytes(self.renderer.layout("<p>Attached</p>").to_pdf(
            attachments={"source.txt": b"source payload"},
            creation_date="D:20261005090000Z",
            modification_date="D:20261005100000Z"))
        reader = PdfReader(io.BytesIO(pdf))
        root = reader.trailer["/Root"]
        names = root["/Names"]["/EmbeddedFiles"]["/Names"]
        self.assertEqual(str(names[0]), "source.txt")
        embedded = names[1].get_object()["/EF"]["/F"].get_object().get_data()
        self.assertEqual(embedded, b"source payload")
        info = reader.metadata
        self.assertEqual(info.creation_date.year, 2026)
        self.assertEqual(info.modification_date.hour, 10)

    def test_pdf_page_boxes_and_crop_marks_are_independent_of_trim_content(self):
        from pypdf import PdfReader

        pdf = bytes(self.renderer.layout("<p>Trimmed</p>",
            "@page { size:200pt 100pt; margin:10pt; bleed:5pt; marks:crop }").to_pdf())
        page = PdfReader(io.BytesIO(pdf)).pages[0]
        self.assertEqual(float(page.mediabox.width), 234.0)
        self.assertEqual(float(page.mediabox.height), 134.0)
        self.assertEqual(float(page.trimbox.width), 200.0)
        self.assertEqual(float(page.bleedbox.width), 210.0)
        self.assertIn(b" m ", page.get_contents().get_data())

    def test_invalid_pdf_date_is_rejected(self):
        with self.assertRaises(ValueError):
            self.renderer.layout("<p>Date</p>").to_pdf(creation_date="yesterday")

    def test_pdf_bytes_match_cross_platform_determinism_fixture(self):
        fonts = serpentype.FontRegistry()
        fonts.register_file(serpentype.bundled_font_path(), "Noto Sans")
        doc = serpentype.Renderer(fonts=fonts).layout(
            "<p>Repeatable PDF 2026</p>",
            "@page { size:200pt 100pt; margin:10pt }")
        digest = hashlib.sha256(bytes(doc.to_pdf())).hexdigest()
        self.assertEqual(digest, "2df1225a1772d9920d1059a3999f4b5cca511ad85f65fc82f4f17443bfd38a9a")

    def test_paginated_pdf_matches_visual_reference(self):
        self.assert_visual_reference("visual_release_reference")

    def test_layout_modes_match_visual_reference(self):
        self.assert_visual_reference("visual_layout_reference")

    def test_raster_and_vector_images_match_visual_reference(self):
        self.assert_visual_reference("visual_resources_reference", base_dir=True)

    def test_repeating_table_header_matches_second_page_visual_reference(self):
        self.assert_visual_reference(
            "visual_table_reference", page=2, expected_text="Item"
        )

    def test_rowspan_and_nested_tables_match_visual_reference(self):
        self.assert_visual_reference(
            "visual_table_features_reference",
            expected_text="Connected and nested table cells",
        )

    def test_first_left_right_and_blank_pages_match_visual_references(self):
        for page, text in (
            (1, ("First page", "First page")),
            (2, ("LEFT", "Quarterly report", "FIXED PAGE MARK")),
            (3, ("BLANK SIDE", "FIXED PAGE MARK")),
            (4, ("LEFT", "Quarterly report", "FIXED PAGE MARK")),
        ):
            with self.subTest(page=page):
                self.assert_visual_reference(
                    "visual_page_selector_reference", page=page, expected_text=text
                )

    def test_typography_colors_visibility_and_borders_match_visual_reference(self):
        self.assert_visual_reference(
            "visual_typography_reference", expected_text="Typography and paint"
        )

    def test_font_fallback_and_shaping_match_visual_reference(self):
        self.assert_visual_reference(
            "visual_fonts_reference", expected_text="Font rendering"
        )

    def test_woff2_face_matches_visual_reference(self):
        self.assert_visual_reference(
            "visual_webfont_reference", base_dir=True, expected_text="Embedded web font"
        )

    def test_paragraph_page_continuation_matches_visual_references(self):
        for page, text in ((1, "Paragraph pagination"), (2, "Following content")):
            with self.subTest(page=page):
                self.assert_visual_reference(
                    "visual_pagination_reference", page=page, expected_text=text
                )

    def assert_visual_reference(self, name, *, base_dir=False, page=1, expected_text=None):
        import subprocess
        import tempfile
        from PIL import Image, ImageChops

        fixtures = Path(__file__).parent / "fixtures"
        html = fixtures.joinpath(f"{name}.html").read_text()
        css = fixtures.joinpath(f"{name}.css").read_text()
        fonts = self.visual_reference_fonts()
        if name == "visual_fonts_reference":
            fonts.register_file(str(fixtures / "NotoEmoji-VF.ttf"), "Noto Emoji")
        renderer = serpentype.Renderer(
            fonts=fonts,
            base_dir=str(fixtures) if base_dir else ".",
            source_name=f"{name}.html",
        )
        pdf = bytes(renderer.layout(html, css).to_pdf())
        from pypdf import PdfReader
        pdf_pages = PdfReader(io.BytesIO(pdf)).pages
        self.assertGreaterEqual(len(pdf_pages), page)
        extracted = pdf_pages[page - 1].extract_text() or ""
        expected_text = {
            "visual_release_reference": "Release preview",
            "visual_layout_reference": "Layout regression",
            "visual_resources_reference": "Resource regression",
            "visual_table_reference": "Repeated table header",
        }.get(name) if expected_text is None else expected_text
        expected_texts = expected_text if isinstance(expected_text, tuple) else (expected_text,)
        for expected in expected_texts:
            self.assertIn(expected, extracted)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "page"
            subprocess.run(
                ["pdftoppm", "-f", str(page), "-singlefile", "-png", "-r", "96", "-", str(output)],
                input=pdf,
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
            )
            with Image.open(output.with_suffix(".png")) as rendered, Image.open(
                fixtures / (f"{name}-page-{page}.png" if page > 1 else f"{name}.png")
            ) as reference:
                rendered = rendered.convert("RGB")
                reference = reference.convert("RGB")
                self.assertEqual(rendered.size, reference.size)
                histogram = ImageChops.difference(rendered, reference).convert("L").histogram()
                pixels = rendered.width * rendered.height
                mean_delta = sum(index * count for index, count in enumerate(histogram)) / pixels
                changed_fraction = sum(histogram[9:]) / pixels
                self.assertLessEqual(mean_delta, 3.0)
                self.assertLessEqual(changed_fraction, 0.05)

    def test_pdf_metadata_from_html_head(self):
        html = ("<html lang='ru'><head><title>Invoice</title>"
                "<meta name='author' content='Alice'>"
                "<meta name='subject' content='Quarterly report'>"
                "<meta name='keywords' content='finance,2026'>"
                "</head><body><p>Text</p></body></html>")
        pdf = bytes(self.renderer.layout(html).to_pdf())
        self.assertIn(b"/Title <FEFF0049006E0076006F006900630065>", pdf)
        self.assertIn(b"/Author <FEFF0041006C006900630065>", pdf)
        self.assertIn(b"/Lang <FEFF00720075>", pdf)

    @staticmethod
    def renderer_fonts():
        fonts = serpentype.FontRegistry()
        fonts.register_file(serpentype.bundled_font_path(), "Noto Sans")
        return fonts

    @staticmethod
    def visual_reference_fonts():
        fonts = serpentype.FontRegistry()
        fonts.register_file(serpentype.bundled_font_path(), "Noto Sans")
        fonts.register_file(serpentype.bundled_font_path(700), "Noto Sans", 700)
        return fonts


if __name__ == "__main__":
    unittest.main()
