//! Installed fonts by family name, for the desktop app and PDF output.
//!
//! The font catalog (`[25].[5]`, FORMAT.md §12) names families as CSS
//! lists ("arial,sans-serif", "Helvetica Neue"). [`find`] looks each one
//! up among the system fonts through fontique (DirectWrite on Windows,
//! CoreText on macOS, fontconfig on Linux, which cover the system and
//! user font folders), and, like FileMaker, substitutes the system's
//! default sans-serif font when none is installed. The web build has no
//! access to system fonts: [`find`] returns `None` there.

use std::sync::Arc;

/// A font face found for a family.
#[derive(Clone)]
pub struct Face {
    /// Family name as installed, e.g. "Arial".
    pub family: String,
    /// The font file (memory-mapped) and the face's index in it (collections).
    pub data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    pub index: u32,
    /// The family has no bold (italic) face: draw this one emboldened (slanted).
    pub fake_bold: bool,
    pub fake_italic: bool,
}

impl std::fmt::Debug for Face {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Face")
            .field("family", &self.family)
            .field("index", &self.index)
            .field("fake_bold", &self.fake_bold)
            .field("fake_italic", &self.fake_italic)
            .finish()
    }
}

/// The installed face for family list `families` in the given weight and
/// slant, or the default sans-serif font's. Results are cached.
#[cfg(target_arch = "wasm32")]
pub fn find(_families: &str, _bold: bool, _italic: bool) -> Option<Face> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn find(families: &str, bold: bool, italic: bool) -> Option<Face> {
    use fontique::{
        Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, GenericFamily,
        QueryFamily, QueryStatus, SourceCache,
    };
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    type Cache = (
        Collection,
        SourceCache,
        HashMap<(String, bool, bool), Option<Face>>,
    );
    static FONTS: OnceLock<Mutex<Cache>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        Mutex::new((
            Collection::new(CollectionOptions {
                shared: false,
                system_fonts: true,
            }),
            SourceCache::default(),
            HashMap::new(),
        ))
    });
    let mut guard = fonts.lock().unwrap_or_else(|e| e.into_inner());
    let (collection, source_cache, cache) = &mut *guard;
    let key = (families.to_string(), bold, italic);
    if let Some(f) = cache.get(&key) {
        return f.clone();
    }
    let mut list: Vec<QueryFamily> = families
        .split(',')
        .map(|n| n.trim().trim_matches(['"', '\'']))
        .filter(|n| !n.is_empty())
        .map(|n| GenericFamily::parse(n).map_or(QueryFamily::Named(n), QueryFamily::Generic))
        .collect();
    list.push(QueryFamily::Generic(GenericFamily::SansSerif));
    let mut query = collection.query(source_cache);
    query.set_families(list);
    query.set_attributes(Attributes {
        width: FontWidth::NORMAL,
        style: if italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        weight: if bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
    });
    let mut found = None;
    query.matches_with(|f| {
        found = Some(f.clone());
        QueryStatus::Stop
    });
    drop(query);
    let face = found.map(|f| {
        let s = f.synthesis;
        // A variable font's default instance is its regular face.
        let vars = !s.variation_settings().is_empty();
        let (fake_bold, fake_italic) = (
            bold && (s.embolden() || vars),
            italic && (s.skew().is_some() || vars),
        );
        let (data, _) = f.blob.into_raw_parts();
        Face {
            family: collection
                .family_name(f.family.0)
                .unwrap_or("Font")
                .to_string(),
            data,
            index: f.index,
            fake_bold,
            fake_italic,
        }
    });
    cache.insert(key, face.clone());
    face
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arial ships with Windows: found by its lower-case catalog name,
    /// with a real bold face. Skipped where it is not installed.
    #[test]
    fn finds_installed_families() {
        if !std::path::Path::new("C:/Windows/Fonts/arial.ttf").exists() {
            return;
        }
        let f = find("arial,sans-serif", false, false).unwrap();
        assert_eq!(f.family, "Arial");
        assert!(!f.fake_bold);
        let b = find("arial,sans-serif", true, false).unwrap();
        assert!(!b.fake_bold);
        assert_ne!(f.data.as_ref().as_ref().len(), 0);
        // A family that is not installed gets the default sans-serif font.
        let d = find("No Such Font Family", false, false).unwrap();
        assert!(!d.family.is_empty());
    }
}
