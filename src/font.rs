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
    metrics: Mutex<HashMap<char, (u16, u16)>>,
}
impl FontData {
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
        let face = ttf_parser::Face::parse(&self.bytes, 0)
            .map_err(|e| Error(format!("font parse failed: {e:?}")))?;
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
#[derive(Default)]
struct Registry {
    cache: Arc<Mutex<Cache>>,
    fonts: Vec<RegisteredFont>,
    limit: usize,
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
        if !matches!(style, "normal" | "italic") {
            return Err(Error(format!("unsupported font style: {style}")));
        }
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
            if face.is_variable() {
                return Err(Error("variable fonts are not supported in v0.1".into()));
            }
            if face.tables().glyf.is_none() {
                return Err(Error(
                    "CFF OpenType fonts are not supported; use TTF outlines".into(),
                ));
            }
            let units_per_em = face.units_per_em();
            let ascent = face.ascender();
            let descent = face.descender();
            let bbox = face.global_bounding_box();
            let data = Arc::new(FontData {
                bytes: Arc::new(bytes),
                digest,
                units_per_em,
                ascent,
                descent,
                bbox,
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
    /// Find the first declared family with the exact weight, style and required glyph.
    pub fn resolve(
        &self,
        families: &[String],
        weight: u16,
        style: &str,
        ch: char,
    ) -> Result<Arc<FontData>> {
        let registry = self.inner.lock().map_err(|e| Error(e.to_string()))?;
        for family in families {
            if let Some(found) = registry.fonts.iter().find(|f| {
                f.family.eq_ignore_ascii_case(family)
                    && f.weight == weight
                    && f.style == style
                    && f.data.has_glyph(ch)
            }) {
                return Ok(found.data.clone());
            }
        }
        Err(Error(format!(
            "no registered font for U+{:04X} ({ch}); requested {} weight {weight} {style}",
            ch as u32,
            families.join(", ")
        )))
    }
    /// Drop evictable cache ownership without invalidating registered or prepared fonts.
    pub fn clear_cache(&self) {
        if let Ok(r) = self.inner.lock() {
            if let Ok(mut cache) = r.cache.lock() {
                cache.entries.clear();
                cache.order.clear();
                cache.bytes = 0;
            }
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
