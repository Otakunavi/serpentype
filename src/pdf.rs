//! Direct PDF serialization of the prepared display list; no layout or file I/O occurs here.
use crate::font::FontData;
use crate::layout::{ImageData, Item, PreparedDocument};
use crate::{Error, Result};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use pdf_writer::Ref;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use subsetter::GlyphRemapper;
const EPS: f32 = 0.02;
type FontUsage = HashMap<[u8; 32], (Arc<FontData>, BTreeMap<u16, String>)>;

#[derive(Clone, Debug)]
pub struct PdfAttachment {
    pub file_name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct PdfOptions {
    pub attachments: Vec<PdfAttachment>,
    /// PDF date string in UTC form `D:YYYYMMDDHHmmSSZ`.
    pub creation_date: Option<String>,
    /// PDF date string in UTC form `D:YYYYMMDDHHmmSSZ`.
    pub modification_date: Option<String>,
}

fn page_extra(style: &crate::css::PageStyle) -> f32 {
    // The page box grows by the declared bleed only. Crop marks are painting
    // instructions and must not silently add another 12pt to the PDF page.
    // This follows WeasyPrint's MediaBox/BleedBox semantics and keeps the
    // selected page dimensions stable when marks are enabled.
    style.bleed
}

fn valid_pdf_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 17
        && &bytes[..2] == b"D:"
        && bytes[2..16].iter().all(u8::is_ascii_digit)
        && bytes[16] == b'Z'
}

fn mime_pdf_name(value: &str) -> Option<String> {
    let (major, minor) = value.split_once('/')?;
    let token = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'!' | b'#' | b'$' | b'&' | b'-' | b'^' | b'_' | b'.' | b'+'
                    )
            })
    };
    (token(major) && token(minor)).then(|| format!("{major}#2F{minor}"))
}

fn crop_mark_ops(ops: &mut String, width: f32, height: f32, bleed: f32, extra: f32) {
    let start = extra - bleed - 10.0;
    let end = extra - bleed - 2.0;
    let right_start = extra + width + bleed + 2.0;
    let right_end = extra + width + bleed + 10.0;
    let bottom = extra;
    let top = extra + height;
    let _ = writeln!(ops, "q 0.25 w 0 G");
    for y in [bottom, top] {
        let _ = writeln!(ops, "{start:.3} {y:.3} m {end:.3} {y:.3} l S");
        let _ = writeln!(ops, "{right_start:.3} {y:.3} m {right_end:.3} {y:.3} l S");
    }
    for x in [extra, extra + width] {
        let _ = writeln!(ops, "{x:.3} {start:.3} m {x:.3} {end:.3} l S");
        let _ = writeln!(ops, "{x:.3} {right_start:.3} m {x:.3} {right_end:.3} l S");
    }
    ops.push_str("Q\n");
}

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
    fn add_chunk(&mut self, chunk: &pdf_writer::Chunk, root: Ref) -> Result<usize> {
        let mut mapping = HashMap::new();
        for old in chunk.refs() {
            let new = self.reserve();
            mapping.insert(old, Ref::new(new as i32));
        }
        let root = mapping
            .get(&root)
            .copied()
            .ok_or_else(|| Error("SVG PDF chunk has no root object".into()))?;
        let mut dangling = false;
        let renumbered = chunk.renumber(|old| {
            mapping.get(&old).copied().unwrap_or_else(|| {
                dangling = true;
                Ref::new(1)
            })
        });
        if dangling {
            return Err(Error(
                "SVG PDF chunk has a dangling object reference".into(),
            ));
        }
        let bytes = renumbered.as_bytes();
        let refs: Vec<_> = renumbered.refs().collect();
        let mut cursor = 0;
        for (index, object) in refs.iter().enumerate() {
            let header = format!("{} 0 obj\n", object.get());
            let start = cursor
                + bytes[cursor..]
                    .windows(header.len())
                    .position(|window| window == header.as_bytes())
                    .ok_or_else(|| Error("invalid SVG PDF object header".into()))?
                + header.len();
            let end_bound = refs.get(index + 1).and_then(|next| {
                let next_header = format!("{} 0 obj\n", next.get());
                bytes[start..]
                    .windows(next_header.len())
                    .position(|window| window == next_header.as_bytes())
                    .map(|offset| start + offset)
            });
            let slice = &bytes[start..end_bound.unwrap_or(bytes.len())];
            let end = slice
                .windows(b"endobj".len())
                .rposition(|window| window == b"endobj")
                .ok_or_else(|| Error("invalid SVG PDF object body".into()))?;
            self.set(
                object.get() as usize,
                slice[..end].trim_ascii_end().to_vec(),
            );
            cursor = end_bound.unwrap_or(bytes.len());
        }
        Ok(root.get() as usize)
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
    if let Some(svg) = &data.svg {
        return pdf.add_chunk(&svg.chunk, svg.root);
    }
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
fn alpha_key(alpha: f32) -> u16 {
    (alpha.clamp(0.0, 1.0) * 10_000.0).round() as u16
}
fn set_alpha(out: &mut String, alpha: f32) {
    if alpha < 1.0 - EPS {
        let _ = writeln!(out, "/GS{} gs", alpha_key(alpha));
    }
}
fn rounded_rect(out: &mut String, x: f32, y: f32, w: f32, h: f32, radius: [f32; 4]) {
    let mut r = radius.map(|value| value.max(0.0));
    let scale = 1.0_f32
        .min(w / (r[0] + r[1]).max(EPS))
        .min(w / (r[3] + r[2]).max(EPS))
        .min(h / (r[0] + r[3]).max(EPS))
        .min(h / (r[1] + r[2]).max(EPS));
    r.iter_mut().for_each(|value| *value *= scale);
    if r.iter().all(|value| *value <= EPS) {
        let _ = write!(out, "{x:.3} {y:.3} {w:.3} {h:.3} re ");
        return;
    }
    let [tl, tr, br, bl] = r;
    const K: f32 = 0.552_284_8;
    let _ = write!(out, "{:.3} {y:.3} m {:.3} {y:.3} l ", x + bl, x + w - br);
    let _ = write!(
        out,
        "{:.3} {y:.3} {:.3} {:.3} {:.3} {:.3} c ",
        x + w - br + br * K,
        x + w,
        y + br - br * K,
        x + w,
        y + br
    );
    let _ = write!(
        out,
        "{:.3} {:.3} l {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c ",
        x + w,
        y + h - tr,
        x + w,
        y + h - tr + tr * K,
        x + w - tr + tr * K,
        y + h,
        x + w - tr,
        y + h
    );
    let _ = write!(
        out,
        "{:.3} {:.3} l {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c ",
        x + tl,
        y + h,
        x + tl - tl * K,
        y + h,
        x,
        y + h - tl + tl * K,
        x,
        y + h - tl
    );
    let _ = write!(
        out,
        "{x:.3} {:.3} l {x:.3} {:.3} {:.3} {y:.3} {:.3} {y:.3} c h ",
        y + bl,
        y + bl - bl * K,
        x + bl - bl * K,
        x + bl
    );
}

/// Export exactly the existing pages, positions, fonts and images.
pub fn export(doc: &PreparedDocument) -> Result<Vec<u8>> {
    export_with_options(doc, &PdfOptions::default(), None)
}
fn affine_multiply(left: [f32; 6], right: [f32; 6]) -> [f32; 6] {
    [
        left[0] * right[0] + left[2] * right[1],
        left[1] * right[0] + left[3] * right[1],
        left[0] * right[2] + left[2] * right[3],
        left[1] * right[2] + left[3] * right[3],
        left[0] * right[4] + left[2] * right[5] + left[4],
        left[1] * right[4] + left[3] * right[5] + left[5],
    ]
}
fn transformed_point(matrix: [f32; 6], x: f32, y: f32) -> (f32, f32) {
    (
        matrix[0] * x + matrix[2] * y + matrix[4],
        matrix[1] * x + matrix[3] * y + matrix[5],
    )
}
fn transformed_rect(matrix: [f32; 6], x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let points = [
        transformed_point(matrix, x, y),
        transformed_point(matrix, x + w, y),
        transformed_point(matrix, x, y + h),
        transformed_point(matrix, x + w, y + h),
    ];
    let left = points
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min);
    let right = points
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let top = points
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min);
    let bottom = points
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max);
    (left, top, right - left, bottom - top)
}
pub fn export_with_cancel(doc: &PreparedDocument, token: Option<&AtomicBool>) -> Result<Vec<u8>> {
    export_with_options(doc, &PdfOptions::default(), token)
}
pub fn export_with_options(
    doc: &PreparedDocument,
    options: &PdfOptions,
    token: Option<&AtomicBool>,
) -> Result<Vec<u8>> {
    let check_cancel = || -> Result<()> {
        if token.is_some_and(|t| t.load(Ordering::Relaxed)) {
            Err(Error("render cancelled".into()))
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    if options
        .creation_date
        .as_deref()
        .is_some_and(|value| !valid_pdf_date(value))
        || options
            .modification_date
            .as_deref()
            .is_some_and(|value| !valid_pdf_date(value))
    {
        return Err(Error(
            "PDF dates must use UTC form D:YYYYMMDDHHmmSSZ".into(),
        ));
    }
    let mut attachment_bytes = 0usize;
    for attachment in &options.attachments {
        if attachment.file_name.trim().is_empty()
            || attachment.file_name.chars().any(char::is_control)
            || mime_pdf_name(&attachment.mime_type).is_none()
        {
            return Err(Error("invalid PDF attachment name or MIME type".into()));
        }
        attachment_bytes = attachment_bytes.saturating_add(attachment.data.len());
        if attachment_bytes > 100_000_000 {
            return Err(Error("PDF attachments exceed 100 MB".into()));
        }
    }
    let mut fonts: FontUsage = HashMap::new();
    let mut images: HashMap<[u8; 32], Arc<ImageData>> = HashMap::new();
    let mut alphas = BTreeSet::new();
    for page in &doc.pages {
        check_cancel()?;
        for item in &page.items {
            match item {
                Item::BeginOpacity(alpha) | Item::RoundedRect { alpha, .. } => {
                    if *alpha < 1.0 - EPS {
                        alphas.insert(alpha_key(*alpha));
                    }
                }
                Item::Border { alphas: values, .. } => {
                    for alpha in values {
                        if *alpha < 1.0 - EPS {
                            alphas.insert(alpha_key(*alpha));
                        }
                    }
                }
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
    let mut alpha_ids = BTreeMap::new();
    for key in &alphas {
        let value = *key as f32 / 10_000.0;
        let id =
            pdf.add(format!("<< /Type /ExtGState /ca {value:.4} /CA {value:.4} >>").into_bytes());
        alpha_ids.insert(*key, id);
    }
    let mut attachments: Vec<_> = options.attachments.iter().collect();
    attachments.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    if attachments
        .windows(2)
        .any(|pair| pair[0].file_name == pair[1].file_name)
    {
        return Err(Error("duplicate PDF attachment file name".into()));
    }
    let mut attachment_specs = Vec::new();
    for attachment in attachments {
        check_cancel()?;
        let mime = mime_pdf_name(&attachment.mime_type)
            .ok_or_else(|| Error("invalid PDF attachment MIME type".into()))?;
        let stream = pdf.stream(
            &format!("/Type /EmbeddedFile /Subtype /{mime}"),
            &attachment.data,
            true,
        )?;
        let file_name = pdf_text(&attachment.file_name);
        let spec = pdf.add(
            format!(
                "<< /Type /Filespec /F {file_name} /UF {file_name} /AFRelationship /Data /EF << /F {stream} 0 R /UF {stream} 0 R >> >>"
            )
            .into_bytes(),
        );
        attachment_specs.push((attachment.file_name.clone(), spec));
    }
    let page_ids: Vec<_> = doc.pages.iter().map(|_| pdf.reserve()).collect();
    let mut destinations = BTreeMap::new();
    for (page_index, page) in doc.pages.iter().enumerate() {
        let mut transforms = vec![[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]];
        for item in &page.items {
            match item {
                Item::BeginTransform(matrix) => {
                    transforms.push(affine_multiply(*transforms.last().unwrap(), *matrix));
                }
                Item::EndTransform => {
                    if transforms.len() > 1 {
                        transforms.pop();
                    }
                }
                Item::Destination { name, x, y } => {
                    let (x, y) = transformed_point(*transforms.last().unwrap(), *x, *y);
                    let extra = page_extra(&page.style);
                    destinations.entry(name.clone()).or_insert((
                        page_ids[page_index],
                        x + extra,
                        page.style.height - y + extra,
                    ));
                }
                _ => {}
            }
        }
    }
    for (page_index, page) in doc.pages.iter().enumerate() {
        check_cancel()?;
        let mut ops = String::new();
        let extra = page_extra(&page.style);
        if extra > 0.0 {
            let _ = writeln!(ops, "q 1 0 0 1 {extra:.3} {extra:.3} cm");
        }
        let mut annotations = Vec::new();
        let mut transforms = vec![[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]];
        let mut position = 0;
        while position < page.items.len() {
            if position % 1024 == 0 {
                check_cancel()?;
            }
            match &page.items[position] {
                Item::BeginTransform(matrix) => {
                    transforms.push(affine_multiply(*transforms.last().unwrap(), *matrix));
                    let [a, b, c, d, e, f] = *matrix;
                    let _ = writeln!(
                        ops,
                        "q {a:.6} {:.6} {:.6} {d:.6} {:.6} {:.6} cm",
                        -b,
                        -c,
                        c * page.style.height + e,
                        page.style.height * (1.0 - d) - f
                    );
                }
                Item::EndTransform => {
                    if transforms.len() > 1 {
                        transforms.pop();
                    }
                    ops.push_str("Q\n");
                }
                Item::BeginActualText(value) => {
                    let _ = writeln!(ops, "/Span << /ActualText {} >> BDC", pdf_text(value));
                }
                Item::EndActualText => ops.push_str("EMC\n"),
                Item::BeginOpacity(alpha) => {
                    ops.push_str("q\n");
                    set_alpha(&mut ops, *alpha);
                }
                Item::EndOpacity => ops.push_str("Q\n"),
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
                Item::RoundedRect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    radius,
                    alpha,
                } => {
                    ops.push_str("q\n");
                    set_color(&mut ops, *fill, false);
                    set_alpha(&mut ops, *alpha);
                    rounded_rect(&mut ops, *x, page.style.height - y - h, *w, *h, *radius);
                    ops.push_str("f\nQ\n");
                }
                Item::Border {
                    x,
                    y,
                    w,
                    h,
                    widths,
                    colors,
                    alphas,
                    styles,
                    radius,
                } => {
                    let bottom = page.style.height - y - h;
                    let edges = [
                        (*x, bottom + *h, *x + *w, bottom + *h),
                        (*x + *w, bottom + *h, *x + *w, bottom),
                        (*x, bottom, *x + *w, bottom),
                        (*x, bottom + *h, *x, bottom),
                    ];
                    for side in 0..4 {
                        if widths[side] <= 0.0 || styles[side] == "none" {
                            continue;
                        }
                        ops.push_str("q\n");
                        set_color(&mut ops, colors[side], true);
                        set_alpha(&mut ops, alphas[side]);
                        let dash = if styles[side] == "dashed" {
                            format!("[{:.3} {:.3}] 0 d", widths[side] * 3.0, widths[side] * 2.0)
                        } else if styles[side] == "dotted" {
                            format!("[0 {:.3}] 0 d 1 J", widths[side] * 2.0)
                        } else {
                            "[] 0 d".into()
                        };
                        let _ = writeln!(ops, "{:.3} w {dash}", widths[side]);
                        if radius.iter().any(|r| *r > EPS) {
                            rounded_rect(&mut ops, *x, bottom, *w, *h, *radius);
                            ops.push_str("W n\n");
                        }
                        let (x1, y1, x2, y2) = edges[side];
                        let _ = writeln!(ops, "{x1:.3} {y1:.3} m {x2:.3} {y2:.3} l S\nQ");
                    }
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
                Item::BeginClip { x, y, w, h, radius } => {
                    ops.push_str("q\n");
                    rounded_rect(&mut ops, *x, page.style.height - y - h, *w, *h, *radius);
                    ops.push_str("W n\n");
                }
                Item::EndClip => ops.push_str("Q\n"),
                Item::Link { x, y, w, h, target } => {
                    let (x, y, w, h) =
                        transformed_rect(*transforms.last().unwrap(), *x, *y, *w, *h);
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
                        let extra = page_extra(&page.style);
                        let id = pdf.add(format!(
                            "<< /Type /Annot /Subtype /Link /Rect [{:.3} {:.3} {:.3} {:.3}] /Border [0 0 0] {action} >>",
                            x + extra,
                            page.style.height - y - h + extra,
                            x + w + extra,
                            page.style.height - y + extra,
                        ).into_bytes());
                        annotations.push(id);
                    }
                }
                Item::Destination { .. } => {}
            }
            position += 1;
        }
        if extra > 0.0 {
            ops.push_str("Q\n");
        }
        if page.style.crop_marks {
            crop_mark_ops(
                &mut ops,
                page.style.width,
                page.style.height,
                page.style.bleed,
                extra,
            );
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
        let alpha_resource = alpha_ids
            .iter()
            .map(|(key, id)| format!("/GS{key} {id} 0 R"))
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
        let extra = page_extra(&page.style);
        let bleed_box = format!(
            " /TrimBox [{extra:.3} {extra:.3} {:.3} {:.3}] /BleedBox [{:.3} {:.3} {:.3} {:.3}]",
            extra + page.style.width,
            extra + page.style.height,
            extra - page.style.bleed,
            extra - page.style.bleed,
            extra + page.style.width + page.style.bleed,
            extra + page.style.height + page.style.bleed,
        );
        pdf.set(page_ids[page_index], format!("<< /Type /Page /Parent {pages_ref} 0 R /MediaBox [0 0 {:.3} {:.3}]{bleed_box}{rotate} /Resources << /Font << {font_resource} >> /XObject << {image_resource} >> /ExtGState << {alpha_resource} >> >> /Contents {content} 0 R{annots} >>",page.style.width + 2.0 * extra,page.style.height + 2.0 * extra).into_bytes());
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
    let destination_names = if destinations.is_empty() {
        String::new()
    } else {
        let entries = destinations
            .iter()
            .map(|(name, (page_id, x, y))| {
                format!("{} [{page_id} 0 R /XYZ {x:.3} {y:.3} null]", pdf_text(name))
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!("/Dests << /Names [{entries}] >>")
    };
    let embedded_names = if attachment_specs.is_empty() {
        String::new()
    } else {
        let entries = attachment_specs
            .iter()
            .map(|(name, spec)| format!("{} {spec} 0 R", pdf_text(name)))
            .collect::<Vec<_>>()
            .join(" ");
        format!("/EmbeddedFiles << /Names [{entries}] >>")
    };
    let names = match (destination_names.is_empty(), embedded_names.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(" /Names << {destination_names} >>"),
        (true, false) => format!(" /Names << {embedded_names} >>"),
        (false, false) => format!(" /Names << {destination_names} {embedded_names} >>"),
    };
    let associated_files = if attachment_specs.is_empty() {
        String::new()
    } else {
        format!(
            " /AF [{}]",
            attachment_specs
                .iter()
                .map(|(_, spec)| format!("{spec} 0 R"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let outlines_entry = if doc.outlines.is_empty() {
        String::new()
    } else {
        let count = doc.outlines.len();
        let root_index = count;
        let mut children = vec![Vec::<usize>::new(); count + 1];
        let mut parents = vec![root_index; count];
        let mut stack: Vec<(u8, usize)> = Vec::new();
        for (index, outline) in doc.outlines.iter().enumerate() {
            while stack
                .last()
                .is_some_and(|(level, _)| *level >= outline.level)
            {
                stack.pop();
            }
            let parent = stack.last().map_or(root_index, |(_, parent)| *parent);
            parents[index] = parent;
            children[parent].push(index);
            stack.push((outline.level, index));
        }
        let mut descendant_counts = vec![0usize; count];
        for index in (0..count).rev() {
            descendant_counts[index] = children[index]
                .iter()
                .map(|child| 1 + descendant_counts[*child])
                .sum();
        }
        let root = pdf.reserve();
        let entries: Vec<_> = doc.outlines.iter().map(|_| pdf.reserve()).collect();
        for (index, (entry, object)) in doc.outlines.iter().zip(&entries).enumerate() {
            let (page_id, x, y) = destinations
                .get(&entry.destination)
                .ok_or_else(|| Error("outline destination is missing".into()))?;
            let siblings = &children[parents[index]];
            let sibling_index = siblings
                .iter()
                .position(|sibling| *sibling == index)
                .unwrap();
            let previous = sibling_index
                .checked_sub(1)
                .map(|previous| format!(" /Prev {} 0 R", entries[siblings[previous]]))
                .unwrap_or_default();
            let next = siblings
                .get(sibling_index + 1)
                .map(|next| format!(" /Next {} 0 R", entries[*next]))
                .unwrap_or_default();
            let parent = if parents[index] == root_index {
                root
            } else {
                entries[parents[index]]
            };
            let children_dict = if children[index].is_empty() {
                String::new()
            } else {
                format!(
                    " /First {} 0 R /Last {} 0 R /Count {}",
                    entries[children[index][0]],
                    entries[*children[index].last().unwrap()],
                    descendant_counts[index],
                )
            };
            pdf.set(
                *object,
                format!(
                    "<< /Title {} /Parent {parent} 0 R{previous}{next}{children_dict} /Dest [{page_id} 0 R /XYZ {:.3} {:.3} null] >>",
                    pdf_text(&entry.title),
                    x,
                    y,
                )
                .into_bytes(),
            );
        }
        let root_children = &children[root_index];
        pdf.set(
            root,
            format!(
                "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
                entries[root_children[0]],
                entries[*root_children.last().unwrap()],
                doc.outlines.len(),
            )
            .into_bytes(),
        );
        format!(" /Outlines {root} 0 R /PageMode /UseOutlines")
    };
    let catalog = pdf.add(
        format!("<< /Type /Catalog /Pages {pages_ref} 0 R{lang}{names}{outlines_entry}{associated_files} >>")
            .into_bytes(),
    );
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
    if let Some(value) = &options.creation_date {
        let _ = write!(info_fields, " /CreationDate ({value})");
    }
    if let Some(value) = &options.modification_date {
        let _ = write!(info_fields, " /ModDate ({value})");
    }
    let info = if info_fields.is_empty() {
        None
    } else {
        Some(pdf.add(format!("<<{info_fields} >>").into_bytes()))
    };
    Ok(pdf.finish(catalog, info))
}
