//! Content-addressed font registry with bounded, thread-safe retained bytes.
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Parsed metadata and owned source bytes retained by prepared documents.
#[derive(Debug)]
pub struct FontData {
    pub bytes: Arc<Vec<u8>>,
    pub digest: [u8; 32],
    pub units_per_em: u16,
    pub ascent: i16,
    pub descent: i16,
    pub bbox: ttf_parser::Rect,
    pub weight_axis: Option<(f32, f32, f32)>,
    pub width_axis: Option<(f32, f32, f32)>,
    pub variation_weight: Option<f32>,
    pub variation_width: Option<f32>,
    pub is_variable: bool,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    metrics: Mutex<HashMap<char, (u16, u16)>>,
}
/// One positioned output glyph from the experimental HarfBuzz-compatible shaper.
pub struct ShapedGlyph {
    pub id: u16,
    pub cluster: usize,
    pub unicode: String,
    pub advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}
impl FontData {
    fn selection_ignores(ch: char) -> bool {
        matches!(
            ch,
            '\u{061C}'
                | '\u{200C}'
                | '\u{200D}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FE0E}'
                | '\u{FE0F}'
        ) || ('\u{E0100}'..='\u{E01EF}').contains(&ch)
    }

    /// Whether one face can shape every mapped character in a grapheme cluster.
    pub fn has_cluster(&self, cluster: &str) -> bool {
        cluster
            .chars()
            .all(|ch| Self::selection_ignores(ch) || self.has_glyph(ch))
    }

    pub fn shape(&self, text: &str) -> Result<Vec<ShapedGlyph>> {
        let mut face = rustybuzz::Face::from_slice(&self.bytes, 0)
            .ok_or_else(|| Error("font cannot be opened for shaping".into()))?;
        let mut variations = Vec::with_capacity(2);
        if let Some(value) = self.variation_weight {
            variations.push(rustybuzz::Variation {
                tag: ttf_parser::Tag::from_bytes(b"wght"),
                value,
            });
        }
        if let Some(value) = self.variation_width {
            variations.push(rustybuzz::Variation {
                tag: ttf_parser::Tag::from_bytes(b"wdth"),
                value,
            });
        }
        if !variations.is_empty() {
            face.set_variations(&variations);
        }
        let mut input = rustybuzz::UnicodeBuffer::new();
        input.push_str(text);
        input.guess_segment_properties();
        let output = rustybuzz::shape(&face, &[], input);
        let infos = output.glyph_infos();
        let positions = output.glyph_positions();
        let mut starts: Vec<usize> = infos.iter().map(|info| info.cluster as usize).collect();
        starts.sort_unstable();
        starts.dedup();
        starts.push(text.len());
        let mut assigned = std::collections::HashSet::new();
        infos
            .iter()
            .zip(positions)
            .map(|(info, position)| {
                let start = info.cluster as usize;
                let end = starts
                    .get(starts.partition_point(|candidate| *candidate <= start))
                    .copied()
                    .unwrap_or(text.len());
                let unicode = if assigned.insert(start) {
                    text.get(start..end)
                        .ok_or_else(|| Error("shaping returned an invalid text cluster".into()))?
                        .to_owned()
                } else {
                    String::new()
                };
                Ok(ShapedGlyph {
                    id: u16::try_from(info.glyph_id)
                        .map_err(|_| Error("shaping returned an invalid glyph ID".into()))?,
                    cluster: start,
                    unicode,
                    advance: position.x_advance,
                    x_offset: position.x_offset,
                    y_offset: position.y_offset,
                })
            })
            .collect()
    }
    pub fn glyph(&self, ch: char) -> Result<(u16, u16)> {
        if let Some(hit) = self
            .metrics
            .lock()
            .map_err(|e| Error(e.to_string()))?
            .get(&ch)
            .copied()
        {
            return Ok(hit);
        }
        let mut face = ttf_parser::Face::parse(&self.bytes, 0)
            .map_err(|e| Error(format!("font parse failed: {e:?}")))?;
        if let Some(weight) = self.variation_weight {
            face.set_variation(ttf_parser::Tag::from_bytes(b"wght"), weight)
                .ok_or_else(|| Error("font weight variation is unavailable".into()))?;
        }
        if let Some(width) = self.variation_width {
            face.set_variation(ttf_parser::Tag::from_bytes(b"wdth"), width)
                .ok_or_else(|| Error("font width variation is unavailable".into()))?;
        }
        let id = face
            .glyph_index(ch)
            .ok_or_else(|| Error(format!("font has no glyph U+{:04X} ({ch})", ch as u32)))?;
        let advance = face.glyph_hor_advance(id).unwrap_or(self.units_per_em);
        let value = (id.0, advance);
        self.metrics
            .lock()
            .map_err(|e| Error(e.to_string()))?
            .insert(ch, value);
        Ok(value)
    }
    pub fn has_glyph(&self, ch: char) -> bool {
        self.glyph(ch).is_ok()
    }
    pub fn width(&self, text: &str, size: f32) -> Result<f32> {
        let mut units = 0u32;
        for ch in text.chars() {
            units += u32::from(self.glyph(ch)?.1);
        }
        Ok(units as f32 * size / f32::from(self.units_per_em))
    }
}

#[derive(Clone, Debug)]
pub struct RegisteredFont {
    pub family: String,
    pub weight: u16,
    pub style: String,
    pub data: Arc<FontData>,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<[u8; 32], Arc<FontData>>,
    order: VecDeque<[u8; 32]>,
    bytes: usize,
    hits: usize,
    misses: usize,
}
type VariantKey = ([u8; 32], u16, u16, bool, bool);

#[derive(Default)]
struct Registry {
    cache: Arc<Mutex<Cache>>,
    fonts: Vec<RegisteredFont>,
    limit: usize,
    variants: HashMap<VariantKey, Arc<FontData>>,
    variant_order: VecDeque<VariantKey>,
    synthetic_bold: bool,
    synthetic_italic: bool,
}
/// Snapshot of cache occupancy and content-addressed lookup counts.
#[derive(Clone, Debug)]
pub struct CacheStats {
    pub bytes: usize,
    pub entries: usize,
    pub hits: usize,
    pub misses: usize,
}
/// Shared registry. Eviction only drops cache ownership; prepared documents keep their Arcs.
#[derive(Clone, Default)]
pub struct FontRegistry {
    inner: Arc<Mutex<Registry>>,
}
impl FontRegistry {
    fn style_distance(requested: &str, available: &str) -> u8 {
        if requested == available {
            0
        } else if matches!(
            (requested, available),
            ("italic", "oblique") | ("oblique", "italic")
        ) {
            1
        } else {
            2
        }
    }
    /// Create a registry whose evictable font bytes are limited by `cache_bytes`.
    pub fn new(cache_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Registry {
                limit: cache_bytes,
                ..Registry::default()
            })),
        }
    }
    /// Copy registrations for one layout so @font-face cannot race another document.
    pub fn snapshot(&self) -> Self {
        let r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self {
            inner: Arc::new(Mutex::new(Registry {
                cache: r.cache.clone(),
                fonts: r.fonts.clone(),
                limit: r.limit,
                variants: r.variants.clone(),
                variant_order: r.variant_order.clone(),
                synthetic_bold: r.synthetic_bold,
                synthetic_italic: r.synthetic_italic,
            })),
        }
    }
    /// Import font bytes from a path immediately; later file changes cannot alter a document.
    pub fn register_file(&self, path: &str, family: &str, weight: u16, style: &str) -> Result<()> {
        let bytes = std::fs::read(path).map_err(|e| Error(format!("font {path}: {e}")))?;
        self.register_bytes(bytes, family, weight, style)
    }
    /// Import a static TrueType-outline font from memory under an explicit family and face.
    pub fn register_bytes(
        &self,
        bytes: Vec<u8>,
        family: &str,
        weight: u16,
        style: &str,
    ) -> Result<()> {
        if !matches!(style, "normal" | "italic" | "oblique") {
            return Err(Error(format!("unsupported font style: {style}")));
        }
        let bytes = if bytes.starts_with(b"wOFF") || bytes.starts_with(b"wOF2") {
            const MAX_DECODED_FONT_BYTES: usize = 100_000_000;
            let declared = bytes
                .get(16..20)
                .and_then(|size| <[u8; 4]>::try_from(size).ok())
                .map(u32::from_be_bytes)
                .unwrap_or(0) as usize;
            if declared == 0 || declared > MAX_DECODED_FONT_BYTES {
                return Err(Error("web font declares an invalid decoded size".into()));
            }
            let decoded = if bytes.starts_with(b"wOFF") {
                wuff::decompress_woff1(&bytes)
            } else {
                wuff::decompress_woff2(&bytes)
            }
            .map_err(|e| Error(format!("web font decode failed: {e}")))?;
            if decoded.len() != declared {
                return Err(Error("web font decoded size does not match header".into()));
            }
            decoded
        } else {
            bytes
        };
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let mut registry = self.inner.lock().map_err(|e| Error(e.to_string()))?;
        let cache_ref = registry.cache.clone();
        let mut cache = cache_ref.lock().map_err(|e| Error(e.to_string()))?;
        let retained = cache.entries.get(&digest).cloned().or_else(|| {
            registry
                .fonts
                .iter()
                .find(|f| f.data.digest == digest)
                .map(|f| f.data.clone())
        });
        let data = if let Some(hit) = retained {
            cache.hits += 1;
            hit
        } else {
            let face = ttf_parser::Face::parse(&bytes, 0)
                .map_err(|e| Error(format!("invalid font: {e:?}")))?;
            if face.tables().glyf.is_none()
                && face.tables().cff.is_none()
                && face.tables().cff2.is_none()
            {
                return Err(Error(
                    "font has no supported TrueType or CFF outlines".into(),
                ));
            }
            let units_per_em = face.units_per_em();
            let ascent = face.ascender();
            let descent = face.descender();
            let bbox = face.global_bounding_box();
            let is_variable = face.is_variable();
            let weight_axis = face
                .variation_axes()
                .into_iter()
                .find(|axis| axis.tag == ttf_parser::Tag::from_bytes(b"wght"))
                .map(|axis| (axis.min_value, axis.def_value, axis.max_value));
            let width_axis = face
                .variation_axes()
                .into_iter()
                .find(|axis| axis.tag == ttf_parser::Tag::from_bytes(b"wdth"))
                .map(|axis| (axis.min_value, axis.def_value, axis.max_value));
            let data = Arc::new(FontData {
                bytes: Arc::new(bytes),
                digest,
                units_per_em,
                ascent,
                descent,
                bbox,
                weight_axis,
                width_axis,
                variation_weight: None,
                variation_width: None,
                is_variable,
                synthetic_bold: false,
                synthetic_italic: false,
                metrics: Mutex::new(HashMap::new()),
            });
            cache.misses += 1;
            data
        };
        if !cache.entries.contains_key(&digest) && data.bytes.len() <= registry.limit {
            while cache.bytes + data.bytes.len() > registry.limit {
                let Some(old) = cache.order.pop_front() else {
                    break;
                };
                if let Some(item) = cache.entries.remove(&old) {
                    cache.bytes -= item.bytes.len();
                }
            }
            cache.bytes += data.bytes.len();
            cache.order.push_back(digest);
            cache.entries.insert(digest, data.clone());
        }
        registry
            .fonts
            .retain(|f| !(f.family == family && f.weight == weight && f.style == style));
        registry.fonts.push(RegisteredFont {
            family: family.into(),
            weight,
            style: style.into(),
            data,
        });
        Ok(())
    }
    /// Return registered font data in registration order for SVG text rendering.
    pub fn source_bytes(&self) -> Result<Vec<Vec<u8>>> {
        let registry = self.inner.lock().map_err(|e| Error(e.to_string()))?;
        let mut seen = std::collections::HashSet::new();
        Ok(registry
            .fonts
            .iter()
            .filter(|font| seen.insert(font.data.digest))
            .map(|font| font.data.bytes.as_ref().clone())
            .collect())
    }
    /// Return unique source digests without cloning font bytes.
    pub fn source_digests(&self) -> Result<Vec<[u8; 32]>> {
        let registry = self.inner.lock().map_err(|e| Error(e.to_string()))?;
        let mut digests: Vec<_> = registry.fonts.iter().map(|font| font.data.digest).collect();
        digests.sort_unstable();
        digests.dedup();
        Ok(digests)
    }
    /// Find the nearest registered face in family order, then a glyph fallback.
    pub fn resolve(
        &self,
        families: &[String],
        weight: u16,
        style: &str,
        ch: char,
    ) -> Result<Arc<FontData>> {
        self.resolve_with_stretch(families, weight, style, 100.0, ch)
    }

    /// Resolve a character with a CSS font-stretch percentage.
    pub fn resolve_with_stretch(
        &self,
        families: &[String],
        weight: u16,
        style: &str,
        stretch: f32,
        ch: char,
    ) -> Result<Arc<FontData>> {
        self.resolve_matching(families, weight, style, stretch, ch, |font| {
            font.has_glyph(ch)
        })
    }

    /// Resolve one face for a complete Unicode grapheme cluster.
    pub fn resolve_cluster(
        &self,
        families: &[String],
        weight: u16,
        style: &str,
        cluster: &str,
    ) -> Result<Arc<FontData>> {
        self.resolve_cluster_with_stretch(families, weight, style, 100.0, cluster)
    }

    /// Resolve one face and width instance for a complete Unicode grapheme cluster.
    pub fn resolve_cluster_with_stretch(
        &self,
        families: &[String],
        weight: u16,
        style: &str,
        stretch: f32,
        cluster: &str,
    ) -> Result<Arc<FontData>> {
        let requested = cluster
            .chars()
            .find(|ch| !FontData::selection_ignores(*ch))
            .ok_or_else(|| Error("font cluster contains no printable character".into()))?;
        self.resolve_matching(families, weight, style, stretch, requested, |font| {
            font.has_cluster(cluster)
        })
    }

    fn resolve_matching(
        &self,
        families: &[String],
        weight: u16,
        style: &str,
        stretch: f32,
        requested: char,
        supports: impl Fn(&FontData) -> bool,
    ) -> Result<Arc<FontData>> {
        let mut registry = self.inner.lock().map_err(|e| Error(e.to_string()))?;
        for family in families {
            let exact = registry.fonts.iter().find(|f| {
                f.family.eq_ignore_ascii_case(family)
                    && f.weight == weight
                    && f.style == style
                    && supports(&f.data)
            });
            if let Some(found) = exact {
                if found.data.weight_axis.is_none()
                    && found.data.width_axis.is_none()
                    && (!registry.synthetic_bold || weight < 600)
                    && (!registry.synthetic_italic || style == "normal")
                {
                    return Ok(found.data.clone());
                }
                let found = found.clone();
                return Self::resolve_variant(&mut registry, found, weight, style, stretch);
            }
            if let Some(found) = registry
                .fonts
                .iter()
                .filter(|f| f.family.eq_ignore_ascii_case(family) && supports(&f.data))
                .min_by_key(|f| {
                    (
                        Self::style_distance(style, &f.style),
                        if let Some((min, _, max)) = f.data.weight_axis {
                            (weight as f32 - (weight as f32).clamp(min, max)).abs() as u16
                        } else {
                            f.weight.abs_diff(weight)
                        },
                    )
                })
                .cloned()
            {
                return Self::resolve_variant(&mut registry, found, weight, style, stretch);
            }
        }
        if let Some(found) = registry
            .fonts
            .iter()
            .filter(|f| supports(&f.data))
            .min_by_key(|f| {
                (
                    Self::style_distance(style, &f.style),
                    f.weight.abs_diff(weight),
                )
            })
            .cloned()
        {
            return Self::resolve_variant(&mut registry, found, weight, style, stretch);
        }
        Err(Error(format!(
            "no registered font for U+{:04X} ({requested}); requested {} weight {weight} {style}",
            requested as u32,
            families.join(", ")
        )))
    }
    fn resolve_variant(
        registry: &mut Registry,
        found: RegisteredFont,
        weight: u16,
        style: &str,
        stretch: f32,
    ) -> Result<Arc<FontData>> {
        let selected = found
            .data
            .weight_axis
            .map(|(min, _, max)| (weight as f32).clamp(min, max));
        let variation_weight = selected.and_then(|value| {
            found
                .data
                .weight_axis
                .and_then(|(_, default, _)| ((value - default).abs() >= 0.01).then_some(value))
        });
        let selected_width = found
            .data
            .width_axis
            .map(|(min, _, max)| stretch.clamp(min, max));
        let variation_width = selected_width.and_then(|value| {
            found
                .data
                .width_axis
                .and_then(|(_, default, _)| ((value - default).abs() >= 0.01).then_some(value))
        });
        let synthetic_bold = registry.synthetic_bold
            && weight >= 600
            && weight > found.weight
            && selected.is_none_or(|value| value < weight as f32);
        let synthetic_italic =
            registry.synthetic_italic && style != "normal" && found.style == "normal";
        if variation_weight.is_none()
            && variation_width.is_none()
            && !synthetic_bold
            && !synthetic_italic
        {
            return Ok(found.data);
        }
        let key = (
            found.data.digest,
            selected.unwrap_or(found.weight as f32) as u16,
            (selected_width.unwrap_or(100.0) * 10.0).round() as u16,
            synthetic_bold,
            synthetic_italic,
        );
        if let Some(cached) = registry.variants.get(&key) {
            return Ok(cached.clone());
        }
        let mut face = ttf_parser::Face::parse(&found.data.bytes, 0)
            .map_err(|e| Error(format!("font parse failed: {e:?}")))?;
        if let Some(value) = variation_weight {
            face.set_variation(ttf_parser::Tag::from_bytes(b"wght"), value)
                .ok_or_else(|| Error("font weight variation is unavailable".into()))?;
        }
        if let Some(value) = variation_width {
            face.set_variation(ttf_parser::Tag::from_bytes(b"wdth"), value)
                .ok_or_else(|| Error("font width variation is unavailable".into()))?;
        }
        let mut hash = Sha256::new();
        hash.update(found.data.digest);
        hash.update(key.1.to_be_bytes());
        hash.update(key.2.to_be_bytes());
        hash.update([u8::from(synthetic_bold), u8::from(synthetic_italic)]);
        let variant = Arc::new(FontData {
            bytes: found.data.bytes.clone(),
            digest: hash.finalize().into(),
            units_per_em: found.data.units_per_em,
            ascent: face.ascender(),
            descent: face.descender(),
            bbox: face.global_bounding_box(),
            weight_axis: found.data.weight_axis,
            width_axis: found.data.width_axis,
            variation_weight,
            variation_width,
            is_variable: found.data.is_variable,
            synthetic_bold,
            synthetic_italic,
            metrics: Mutex::new(HashMap::new()),
        });
        if registry.variants.len() >= 64 {
            if let Some(old) = registry.variant_order.pop_front() {
                registry.variants.remove(&old);
            }
        }
        registry.variant_order.push_back(key);
        registry.variants.insert(key, variant.clone());
        Ok(variant)
    }
    /// Drop evictable cache ownership without invalidating registered or prepared fonts.
    pub fn clear_cache(&self) {
        if let Ok(mut r) = self.inner.lock() {
            r.variants.clear();
            r.variant_order.clear();
            if let Ok(mut cache) = r.cache.lock() {
                cache.entries.clear();
                cache.order.clear();
                cache.bytes = 0;
            }
        }
    }
    /// Enable explicit synthetic faces for layouts using this registry snapshot.
    pub fn set_synthetic(&self, bold: bool, italic: bool) {
        if let Ok(mut registry) = self.inner.lock() {
            registry.synthetic_bold = bold;
            registry.synthetic_italic = italic;
        }
    }
    /// Return current evictable bytes and registration cache hit counts.
    pub fn stats(&self) -> CacheStats {
        let r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let cache = r.cache.lock().unwrap_or_else(|e| e.into_inner());
        CacheStats {
            bytes: cache.bytes,
            entries: cache.entries.len(),
            hits: cache.hits,
            misses: cache.misses,
        }
    }
}
