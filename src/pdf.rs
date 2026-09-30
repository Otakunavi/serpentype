//! Direct PDF serialization of the prepared display list; no layout or file I/O occurs here.
use crate::font::FontData;
use crate::layout::{ImageData, Item, PreparedDocument};
use crate::{Error, Result};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use subsetter::GlyphRemapper;
const EPS: f32 = 0.02;
type FontUsage = HashMap<[u8; 32], (Arc<FontData>, BTreeMap<u16, String>)>;

struct Pdf {
    objects: Vec<Vec<u8>>,
}
impl Pdf {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }
    fn reserve(&mut self) -> usize {
        self.objects.push(Vec::new());
        self.objects.len()
    }
    fn set(&mut self, id: usize, data: Vec<u8>) {
        self.objects[id - 1] = data;
    }
    fn add(&mut self, data: Vec<u8>) -> usize {
        let id = self.reserve();
        self.set(id, data);
        id
    }
    fn stream(&mut self, dict: &str, raw: &[u8], compress: bool) -> Result<usize> {
        let data = if compress {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
            enc.write_all(raw).map_err(|e| Error(e.to_string()))?;
            enc.finish().map_err(|e| Error(e.to_string()))?
        } else {
            raw.to_vec()
        };
        let mut out = format!(
            "<< {dict} /Length {}{} >>\nstream\n",
            data.len(),
            if compress {
                " /Filter /FlateDecode"
            } else {
                ""
            }
        )
        .into_bytes();
        out.extend_from_slice(&data);
        out.extend_from_slice(b"\nendstream");
        Ok(self.add(out))
    }
    fn finish(self, root: usize, info: Option<usize>) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = vec![0usize];
        for (i, obj) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(obj);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for pos in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{pos:010} 00000 n \n").as_bytes());
        }
        let info = info
            .map(|id| format!(" /Info {id} 0 R"))
            .unwrap_or_default();
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R{info} >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        out
    }
}
fn utf16_hex(value: &str) -> String {
    value.encode_utf16().map(|c| format!("{c:04X}")).collect()
}
fn pdf_text(value: &str) -> String {
    let mut encoded = String::from("<FEFF");
    for unit in value.encode_utf16() {
        let _ = write!(encoded, "{unit:04X}");
    }
    encoded.push('>');
    encoded
}
fn to_unicode(entries: &BTreeMap<u16, String>) -> Vec<u8> {
    let mut map=String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /STFUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    for chunk in entries.iter().collect::<Vec<_>>().chunks(100) {
        let _ = writeln!(map, "{} beginbfchar", chunk.len());
        for (gid, value) in chunk {
            let _ = writeln!(map, "<{gid:04X}> <{}>", utf16_hex(value));
        }
        map.push_str("endbfchar\n");
    }
    map.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    map.into_bytes()
}
fn font_objects(
    pdf: &mut Pdf,
    font: &FontData,
    glyphs: &BTreeMap<u16, String>,
    index: usize,
) -> Result<(usize, HashMap<u16, u16>)> {
    // The subsetter assigns dense new glyph IDs. PDF CIDs use these IDs, so
    // content streams, widths and ToUnicode must all use the same remapping.
    let mut remapper = GlyphRemapper::new();
    let mut mapped = HashMap::new();
    let mut subset_tag_hash = Sha256::new();
    subset_tag_hash.update(font.digest);
    for old_id in glyphs.keys() {
        mapped.insert(*old_id, remapper.remap(*old_id));
        subset_tag_hash.update(old_id.to_be_bytes());
    }
    let subset = if font.is_variable {
        let mut variations = Vec::with_capacity(2);
        if let Some(value) = font.variation_weight {
            variations.push((subsetter::Tag::new(b"wght"), value));
        }
        if let Some(value) = font.variation_width {
            variations.push((subsetter::Tag::new(b"wdth"), value));
        }
        subsetter::subset_with_variations(&font.bytes, 0, &variations, &remapper)
    } else {
        subsetter::subset(&font.bytes, 0, &remapper)
    }
    .map_err(|e| Error(format!("font subsetting failed: {e}")))?;
    let cff = subset.starts_with(b"OTTO");
    let subset_tag: String = subset_tag_hash.finalize()[..6]
        .iter()
        .map(|byte| (b'A' + byte % 26) as char)
        .collect();
    let font_name = format!("{subset_tag}+STF{index}");
    let file = if cff {
        pdf.stream("/Subtype /OpenType", &subset, true)?
    } else {
        pdf.stream(&format!("/Length1 {}", subset.len()), &subset, true)?
    };
    let scale = 1000.0 / font.units_per_em as f32;
    let bbox = font.bbox;
    let descriptor=format!("<< /Type /FontDescriptor /FontName /{font_name} /Flags {} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV {} /{} {file} 0 R >>",
        if font.synthetic_italic { 96 } else { 32 },
        (bbox.x_min as f32*scale) as i32,(bbox.y_min as f32*scale) as i32,(bbox.x_max as f32*scale) as i32,(bbox.y_max as f32*scale) as i32,
        if font.synthetic_italic { -12 } else { 0 },
        (font.ascent as f32*scale) as i32,(font.descent as f32*scale) as i32,(font.ascent as f32*scale) as i32,
        if font.synthetic_bold { 120 } else { 80 },
        if cff { "FontFile3" } else { "FontFile2" });
    let descriptor = pdf.add(descriptor.into_bytes());
    let mut face = ttf_parser::Face::parse(&font.bytes, 0)
        .map_err(|e| Error(format!("font parse failed during PDF export: {e:?}")))?;
    if let Some(weight) = font.variation_weight {
        face.set_variation(ttf_parser::Tag::from_bytes(b"wght"), weight)
            .ok_or_else(|| Error("font weight variation is unavailable".into()))?;
    }
    if let Some(width) = font.variation_width {
        face.set_variation(ttf_parser::Tag::from_bytes(b"wdth"), width)
            .ok_or_else(|| Error("font width variation is unavailable".into()))?;
    }
    let mut widths = String::new();
    let mut remapped_glyphs = BTreeMap::new();
    for (old_gid, value) in glyphs {
        let gid = mapped[old_gid];
        let advance = face
            .glyph_hor_advance(ttf_parser::GlyphId(*old_gid))
            .unwrap_or(font.units_per_em) as f32
            * scale;
        let _ = write!(widths, "{gid} [{:.2}] ", advance);
        remapped_glyphs.insert(gid, value.clone());
    }
    let subtype = if cff { "CIDFontType0" } else { "CIDFontType2" };
    let cid_to_gid = if cff { "" } else { " /CIDToGIDMap /Identity" };
    let cid=pdf.add(format!("<< /Type /Font /Subtype /{subtype} /BaseFont /{font_name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R{cid_to_gid} /DW 1000 /W [{widths}] >>").into_bytes());
    let cmap = pdf.stream("", &to_unicode(&remapped_glyphs), true)?;
    let font_id = pdf.add(format!("<< /Type /Font /Subtype /Type0 /BaseFont /{font_name} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {cmap} 0 R >>").into_bytes());
    Ok((font_id, mapped))
}
fn image_object(pdf: &mut Pdf, data: &ImageData) -> Result<usize> {
    let base = format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8",data.width,data.height);
    if let Some(jpeg) = &data.jpeg {
        return pdf.stream(&format!("{base} /Filter /DCTDecode"), jpeg, false);
    }
    let mask = if let Some(alpha) = &data.alpha {
        let id = pdf.stream(&format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8",data.width,data.height), alpha, true)?;
        format!(" /SMask {id} 0 R")
    } else {
        String::new()
    };
    pdf.stream(&format!("{base}{mask}"), &data.rgb, true)
}
fn set_color(out: &mut String, c: crate::css::Color, stroke: bool) {
    let _ = writeln!(
        out,
        "{:.4} {:.4} {:.4} {}",
        c.0,
        c.1,
        c.2,
        if stroke { "RG" } else { "rg" }
    );
}

/// Export exactly the existing pages, positions, fonts and images.
pub fn export(doc: &PreparedDocument) -> Result<Vec<u8>> {
    export_with_cancel(doc, None)
}
pub fn export_with_cancel(doc: &PreparedDocument, token: Option<&AtomicBool>) -> Result<Vec<u8>> {
    let check_cancel = || -> Result<()> {
        if token.is_some_and(|t| t.load(Ordering::Relaxed)) {
            Err(Error("render cancelled".into()))
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    let mut fonts: FontUsage = HashMap::new();
    let mut images: HashMap<[u8; 32], Arc<ImageData>> = HashMap::new();
    for page in &doc.pages {
        check_cancel()?;
        for item in &page.items {
            match item {
                Item::Text {
                    ch,
                    glyph,
                    unicode,
                    font,
                    ..
                } => {
                    let value = unicode
                        .as_deref()
                        .map(str::to_owned)
                        .unwrap_or_else(|| ch.to_string());
                    fonts
                        .entry(font.digest)
                        .or_insert_with(|| (font.clone(), BTreeMap::new()))
                        .1
                        .entry(*glyph)
                        .and_modify(|existing| {
                            if existing.is_empty() && !value.is_empty() {
                                *existing = value.clone();
                            }
                        })
                        .or_insert(value);
                }
                Item::Image { data, .. } => {
                    images.entry(data.digest).or_insert_with(|| data.clone());
                }
                _ => {}
            }
        }
    }
    let mut pdf = Pdf::new();
    let pages_ref = pdf.reserve();
    let mut font_ids = HashMap::new();
    let mut font_keys: Vec<_> = fonts.keys().copied().collect();
    font_keys.sort();
    for (index, key) in font_keys.iter().enumerate() {
        check_cancel()?;
        let (data, glyphs) = &fonts[key];
        let (id, remapping) = font_objects(&mut pdf, data, glyphs, index + 1)?;
        font_ids.insert(*key, (index + 1, id, remapping));
    }
    let mut image_ids = HashMap::new();
    let mut image_keys: Vec<_> = images.keys().copied().collect();
    image_keys.sort();
    for (index, key) in image_keys.iter().enumerate() {
        let id = image_object(&mut pdf, &images[key])?;
        image_ids.insert(*key, (index + 1, id));
    }
    let page_ids: Vec<_> = doc.pages.iter().map(|_| pdf.reserve()).collect();
    let mut destinations = BTreeMap::new();
    for (page_index, page) in doc.pages.iter().enumerate() {
        for item in &page.items {
            if let Item::Destination { name, x, y } = item {
                destinations.entry(name.clone()).or_insert((
                    page_ids[page_index],
                    *x,
                    page.style.height - y,
                ));
            }
        }
    }
    for (page_index, page) in doc.pages.iter().enumerate() {
        check_cancel()?;
        let mut ops = String::new();
        let mut annotations = Vec::new();
        let mut position = 0;
        while position < page.items.len() {
            if position % 1024 == 0 {
                check_cancel()?;
            }
            match &page.items[position] {
                Item::BeginActualText(value) => {
                    let _ = writeln!(ops, "/Span << /ActualText {} >> BDC", pdf_text(value));
                }
                Item::EndActualText => ops.push_str("EMC\n"),
                Item::Text {
                    font,
                    x,
                    y,
                    size,
                    color,
                    ..
                } => {
                    let (index, _, remapping) = font_ids
                        .get(&font.digest)
                        .ok_or_else(|| Error("missing PDF font".into()))?;
                    let mut encoded = String::new();
                    let mut end = position;
                    let mut expected_x = *x;
                    while let Some(Item::Text {
                        glyph,
                        font: next_font,
                        x: next_x,
                        y: next_y,
                        size: next_size,
                        color: next_color,
                        natural_advance,
                        ..
                    }) = page.items.get(end)
                    {
                        if next_font.digest != font.digest
                            || (next_y - y).abs() > EPS
                            || (next_size - size).abs() > EPS
                            || next_color != color
                            || (next_x - expected_x).abs() > EPS
                        {
                            break;
                        }
                        let cid = remapping
                            .get(glyph)
                            .ok_or_else(|| Error("glyph missing from PDF font subset".into()))?;
                        let _ = write!(encoded, "{cid:04X}");
                        expected_x += *natural_advance;
                        end += 1;
                    }
                    if font.synthetic_bold || font.synthetic_italic {
                        ops.push_str("q\n");
                    }
                    set_color(&mut ops, *color, false);
                    if font.synthetic_bold {
                        set_color(&mut ops, *color, true);
                        let _ = writeln!(ops, "{:.3} w", size * 0.03);
                    }
                    let _ = writeln!(
                        ops,
                        "BT /F{index} {:.3} Tf {}1 0 {} 1 {:.3} {:.3} Tm <{encoded}> Tj ET",
                        size,
                        if font.synthetic_bold { "2 Tr " } else { "" },
                        if font.synthetic_italic { "0.220" } else { "0" },
                        x,
                        page.style.height - y
                    );
                    if font.synthetic_bold || font.synthetic_italic {
                        ops.push_str("Q\n");
                    }
                    position = end;
                    continue;
                }
                Item::Rect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    stroke,
                } => {
                    ops.push_str("q\n");
                    if let Some(c) = fill {
                        set_color(&mut ops, *c, false);
                    }
                    if let Some((thickness, c)) = stroke {
                        set_color(&mut ops, *c, true);
                        let _ = writeln!(ops, "{thickness:.3} w");
                    }
                    let _ = writeln!(
                        ops,
                        "{x:.3} {:.3} {w:.3} {h:.3} re {}",
                        page.style.height - y - h,
                        match (fill, stroke) {
                            (Some(_), Some(_)) => "B",
                            (Some(_), None) => "f",
                            (None, Some(_)) => "S",
                            _ => "n",
                        }
                    );
                    ops.push_str("Q\n");
                }
                Item::Image { data, x, y, w, h } => {
                    let (index, _) = image_ids
                        .get(&data.digest)
                        .ok_or_else(|| Error("missing PDF image".into()))?;
                    let _ = writeln!(
                        ops,
                        "q {w:.3} 0 0 {h:.3} {x:.3} {:.3} cm /Im{index} Do Q",
                        page.style.height - y - h
                    );
                }
                Item::Link { x, y, w, h, target } => {
                    let action = if let Some(name) = target.strip_prefix('#') {
                        destinations.get(name).map(|(page_id, dx, dy)| {
                            format!("/Dest [{page_id} 0 R /XYZ {dx:.3} {dy:.3} null]")
                        })
                    } else {
                        let url: String = target
                            .as_bytes()
                            .iter()
                            .map(|b| format!("{b:02X}"))
                            .collect();
                        Some(format!("/A << /S /URI /URI <{url}> >>"))
                    };
                    if let Some(action) = action {
                        let id = pdf.add(format!(
                            "<< /Type /Annot /Subtype /Link /Rect [{x:.3} {:.3} {:.3} {:.3}] /Border [0 0 0] {action} >>",
                            page.style.height - y - h, x + w, page.style.height - y,
                        ).into_bytes());
                        annotations.push(id);
                    }
                }
                Item::Destination { .. } => {}
            }
            position += 1;
        }
        let content = pdf.stream("", ops.as_bytes(), true)?;
        let font_resource = font_keys
            .iter()
            .map(|k| {
                let (n, id, _) = &font_ids[k];
                format!("/F{n} {id} 0 R")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let image_resource = image_keys
            .iter()
            .map(|k| {
                let (n, id) = image_ids[k];
                format!("/Im{n} {id} 0 R")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let rotate = if page.style.rotation == 0 {
            String::new()
        } else {
            format!(" /Rotate {}", page.style.rotation)
        };
        let annots = if annotations.is_empty() {
            String::new()
        } else {
            format!(
                " /Annots [{}]",
                annotations
                    .iter()
                    .map(|id| format!("{id} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        pdf.set(page_ids[page_index], format!("<< /Type /Page /Parent {pages_ref} 0 R /MediaBox [0 0 {:.3} {:.3}]{rotate} /Resources << /Font << {font_resource} >> /XObject << {image_resource} >> >> /Contents {content} 0 R{annots} >>",page.style.width,page.style.height).into_bytes());
    }
    let kids = page_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    pdf.set(
        pages_ref,
        format!(
            "<< /Type /Pages /Count {} /Kids [{kids}] >>",
            page_ids.len()
        )
        .into_bytes(),
    );
    let lang = doc
        .metadata
        .language
        .as_deref()
        .map(|value| format!(" /Lang {}", pdf_text(value)))
        .unwrap_or_default();
    let names = if destinations.is_empty() {
        String::new()
    } else {
        let entries = destinations
            .iter()
            .map(|(name, (page_id, x, y))| {
                format!("{} [{page_id} 0 R /XYZ {x:.3} {y:.3} null]", pdf_text(name))
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(" /Names << /Dests << /Names [{entries}] >> >>")
    };
    let catalog =
        pdf.add(format!("<< /Type /Catalog /Pages {pages_ref} 0 R{lang}{names} >>").into_bytes());
    let mut info_fields = String::new();
    for (key, value) in [
        ("Title", doc.metadata.title.as_deref()),
        ("Author", doc.metadata.author.as_deref()),
        ("Subject", doc.metadata.subject.as_deref()),
        ("Keywords", doc.metadata.keywords.as_deref()),
    ] {
        if let Some(value) = value {
            let _ = write!(info_fields, " /{key} {}", pdf_text(value));
        }
    }
    let info = if info_fields.is_empty() {
        None
    } else {
        Some(pdf.add(format!("<<{info_fields} >>").into_bytes()))
    };
    Ok(pdf.finish(catalog, info))
}
