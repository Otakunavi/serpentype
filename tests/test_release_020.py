"""Regression cases for capabilities added on the road to 0.2.0."""
import base64
import io
import json
import re
import tempfile
import unittest
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

    def test_resource_loader_mapping_and_limits(self):
        payload = Path(__file__).parent.joinpath("fixtures", "alpha.png").read_bytes()
        loader = serpentype.ResourceLoader(mapping={"memory:logo": payload}, max_resources=1)
        resource = loader.load("memory:logo", serpentype.ResourceKind.IMAGE)
        self.assertEqual(resource.data, payload)
        self.assertEqual(loader.load("memory:logo", serpentype.ResourceKind.IMAGE).data, payload)
        with self.assertRaises(serpentype.ResourceLimitError):
            loader.load("data:text/plain,another", serpentype.ResourceKind.IMAGE)

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

    def test_compat_html_filename_is_used_for_missing_glyph_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invoice source.html"
            source.write_text("<p>&#x10fff;</p>", encoding="utf-8")
            with self.assertRaises(serpentype.MissingGlyphError) as caught:
                serpentype.HTML(filename=source).render()
            self.assertEqual(caught.exception.source, str(source))
            self.assertEqual((caught.exception.line, caught.exception.column), (1, 4))

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
            self.assertIn(b"/Subtype /Image", local)
            self.assertIn(b"/SMask", local)
            self.assertIn(b"/Width 180 /Height 120", local)
            self.assertEqual(local, inline)
            low_dpi = serpentype.Renderer(base_dir=str(root), svg_dpi=96)
            self.assertIn(b"/Width 120 /Height 80",
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
        self.assertIn(b"/Subtype /Image", pdf)

    def test_svg_embedded_raster_data_image(self):
        png = (Path(__file__).parent / "fixtures" / "alpha.png").read_bytes()
        png_uri = "data:image/png;base64," + base64.b64encode(png).decode()
        svg = ("<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>"
               f"<image href='{png_uri}' width='20' height='20'/></svg>")
        svg_uri = "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()
        pdf = bytes(self.renderer.layout(f"<img src='{svg_uri}'>").to_pdf())
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


if __name__ == "__main__":
    unittest.main()
