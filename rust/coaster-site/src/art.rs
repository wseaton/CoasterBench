//! Where the site's images come from.
//!
//! Screenshots and previews are heavy, so they live in the R2 artifact store
//! (uploaded by `evals/publish.py`, listed in `artifacts.json` /
//! `library-previews.json`) instead of git. A checkout that still has the
//! local files uses them; CI works from the manifests alone.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A run artifact: a local file, a remote URL in the artifact store, or both.
/// Local wins so dev builds don't touch the network.
#[derive(Debug, Clone)]
pub struct Art {
    pub name: String,
    pub local: Option<PathBuf>,
    pub url: Option<String>,
}

pub struct ArtStore {
    artifact_base: String,
    /// Per-run artifacts.json contents, loaded on first use.
    manifests: RefCell<HashMap<PathBuf, HashSet<String>>>,
    /// File names of previews uploaded to the artifact store.
    previews: HashSet<String>,
    previews_dir: PathBuf,
    /// Downloads of remote-only artifacts, kept across builds: thumbnail and
    /// og-card generation need actual pixels.
    cache_dir: PathBuf,
}

fn read_manifest(path: &Path) -> Result<HashSet<String>> {
    if !path.is_file() {
        return Ok(HashSet::new());
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

impl ArtStore {
    pub fn new(artifact_base: &str, previews_dir: &Path, previews_manifest: &Path) -> Result<Self> {
        Ok(Self {
            artifact_base: artifact_base.trim_end_matches('/').to_string(),
            manifests: RefCell::new(HashMap::new()),
            previews: read_manifest(previews_manifest)?,
            previews_dir: previews_dir.to_path_buf(),
            cache_dir: std::env::temp_dir().join("coaster-site-art"),
        })
    }

    /// Resolves one run-relative artifact path, or None when the file is
    /// neither on disk nor in the run's manifest.
    pub fn art(&self, run_dir: &Path, rel: &Path) -> Option<Art> {
        let name = rel.file_name()?.to_str()?.to_string();
        let local = run_dir.join(rel);
        if local.is_file() {
            return Some(Art {
                name,
                local: Some(local),
                url: None,
            });
        }
        let key = rel.to_str()?.replace('\\', "/");
        let mut manifests = self.manifests.borrow_mut();
        let manifest = match manifests.entry(run_dir.to_path_buf()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let loaded = read_manifest(&run_dir.join("artifacts.json")).unwrap_or_else(|err| {
                    eprintln!("warning: {err:#}");
                    HashSet::new()
                });
                e.insert(loaded)
            }
        };
        if !manifest.contains(&key) {
            return None;
        }
        let run = run_dir.file_name()?.to_str()?;
        Some(Art {
            name,
            local: None,
            url: Some(format!("{}/runs/{run}/{key}", self.artifact_base)),
        })
    }

    /// A track design's preview PNG: the local render, else the artifact store.
    pub fn preview(&self, design_name: &str) -> Option<Art> {
        let file = format!("{}.png", crate::model::sanitise_name(design_name));
        let local = self.previews_dir.join(&file);
        if local.is_file() {
            return Some(Art {
                name: file,
                local: Some(local),
                url: None,
            });
        }
        if !self.previews.contains(&file) {
            return None;
        }
        let url = format!("{}/library-previews/{file}", self.artifact_base);
        Some(Art {
            name: file,
            local: None,
            url: Some(url),
        })
    }

    /// Every preview the site knows about, by design name, for the gallery
    /// page fallback when no library.json has been recorded yet.
    pub fn preview_names(&self) -> Vec<String> {
        let mut names: HashSet<String> = self
            .previews
            .iter()
            .filter_map(|f| f.strip_suffix(".png").map(str::to_string))
            .collect();
        if let Ok(entries) = std::fs::read_dir(&self.previews_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "png")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    names.insert(stem.to_string());
                }
            }
        }
        let mut names: Vec<String> = names.into_iter().collect();
        names.sort();
        names
    }

    pub fn have_previews(&self) -> bool {
        !self.previews.is_empty()
            || std::fs::read_dir(&self.previews_dir).is_ok_and(|mut entries| {
                entries.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "png")))
            })
    }

    /// A path with real pixels behind it, downloading remote-only artifacts.
    /// Returns None (with a warning) when the fetch fails: a missing
    /// thumbnail should not fail the whole build.
    pub fn pixels(&self, art: &Art) -> Option<PathBuf> {
        if let Some(local) = &art.local {
            return Some(local.clone());
        }
        let url = art.url.as_ref()?;
        let file = url.rsplit("://").next().unwrap_or(url);
        let dest = self.cache_dir.join(crate::model::sanitise_name(file));
        if dest.is_file() {
            return Some(dest);
        }
        match download(url, &dest) {
            Ok(()) => Some(dest),
            Err(err) => {
                eprintln!("warning: failed to fetch {url}: {err:#}");
                None
            }
        }
    }
}

fn download(url: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Cloudflare's bot filtering 403s clients with no user agent.
    let bytes = ureq::get(url)
        .header("User-Agent", "coasterbench-site")
        .call()?
        .body_mut()
        .read_to_vec()?;
    std::fs::write(dest, bytes)?;
    Ok(())
}
