//! The layout sidecar (`name.cic.layout.json`, docs/10 §The layout
//! sidecar): manual overrides ONLY — a node never hand-moved has no entry,
//! so the file stays near-empty and near-auto by construction. Deleting an
//! entry (or the file) snaps back to auto-layout; nothing but aesthetics is
//! ever at stake, which is why the differ ignores it and why unknown keys
//! are preserved untouched (forward compatibility).
//!
//! Grid-native: `cell` is integer grid units (one unit = the port-row
//! pitch), never pixels.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The sidecar format version this build writes.
pub const SIDECAR_VERSION: u32 = 1;

/// One node's manual overrides. Every field optional: an override entry
/// exists only for the aspects the user touched.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Override {
    /// Grid cell `[x, y]` (integer units).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<[i64; 2]>,
    /// Node color (any CSS color; purely visual).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Collapsed rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    /// Port display order (a pure UI mapping over the kwargs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_order: Option<Vec<String>>,
    /// Preview toggle (docs/16: eye badge). Absent = the default (on for
    /// geometry outputs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<bool>,
    /// Keys this build does not know — preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Override {
    /// True when nothing is overridden (the entry can be dropped).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cell.is_none()
            && self.color.is_none()
            && self.collapsed.is_none()
            && self.port_order.is_none()
            && self.preview.is_none()
            && self.extra.is_empty()
    }
}

/// The sidecar document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Format version.
    pub version: u32,
    /// Per-node overrides keyed by binding name (identity for layout).
    #[serde(default)]
    pub overrides: BTreeMap<String, Override>,
    /// Groups (purely visual, DECISIONS.md groups row) — carried, not
    /// interpreted in the spike.
    #[serde(default)]
    pub groups: Vec<serde_json::Value>,
    /// Views / camera bookmarks — carried, not interpreted in the spike.
    #[serde(default)]
    pub views: serde_json::Value,
    /// Unknown top-level keys, preserved.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for Sidecar {
    fn default() -> Self {
        Self {
            version: SIDECAR_VERSION,
            overrides: BTreeMap::new(),
            groups: Vec::new(),
            views: serde_json::json!({ "bookmarks": [] }),
            extra: BTreeMap::new(),
        }
    }
}

/// Sidecar I/O failures — loud; a corrupt sidecar is reported, never
/// silently replaced (it may hold hand-tuned layout).
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    /// Reading/writing the file failed.
    #[error("{path}: {source}")]
    Io {
        /// The sidecar path.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// The file exists but is not a sidecar this build understands.
    #[error("{path} is not a valid layout sidecar: {message}")]
    Invalid {
        /// The sidecar path.
        path: PathBuf,
        /// What failed.
        message: String,
    },
}

impl Sidecar {
    /// The sidecar path for a pipeline: `name.cic` → `name.cic.layout.json`.
    #[must_use]
    pub fn path_for(pipeline: &Path) -> PathBuf {
        let mut name = pipeline
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        name.push_str(".layout.json");
        pipeline.with_file_name(name)
    }

    /// Load a sidecar; a missing file is the empty default (near-empty by
    /// construction), an unreadable or malformed one is an error.
    ///
    /// # Errors
    ///
    /// [`SidecarError`].
    pub fn load(path: &Path) -> Result<Self, SidecarError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|message| SidecarError::Invalid {
                path: path.to_owned(),
                message,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(SidecarError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Parse sidecar JSON text.
    ///
    /// # Errors
    ///
    /// The serde message when the text is not a sidecar.
    pub fn parse(text: &str) -> Result<Self, String> {
        let sidecar: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if sidecar.version > SIDECAR_VERSION {
            return Err(format!(
                "sidecar version {} is newer than this build writes ({SIDECAR_VERSION})",
                sidecar.version
            ));
        }
        Ok(sidecar)
    }

    /// Deterministic pretty JSON (`BTreeMap` keys → stable order), trailing
    /// newline. Same content → same bytes.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_owned());
        text.push('\n');
        text
    }

    /// Write the sidecar atomically (temp + rename, like the `.cic`: a
    /// reader never sees a torn sidecar, and a failed write leaves the
    /// file as it was). When every override is empty and nothing else is
    /// set, the file is REMOVED instead (near-empty by construction: no
    /// state, no file).
    ///
    /// # Errors
    ///
    /// [`SidecarError::Io`].
    pub fn save(&self, path: &Path) -> Result<(), SidecarError> {
        let io = |source| SidecarError::Io {
            path: path.to_owned(),
            source,
        };
        if self.is_default_shape() {
            match std::fs::remove_file(path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(source) => return Err(io(source)),
            }
        }
        crate::atomic::write_atomic(path, self.render().as_bytes()).map_err(io)
    }

    fn is_default_shape(&self) -> bool {
        self.overrides.values().all(Override::is_empty)
            && self.groups.is_empty()
            && self.extra.is_empty()
            && self.views == Self::default().views
    }

    /// The override entry for a node, created empty on demand.
    pub fn entry(&mut self, node: &str) -> &mut Override {
        self.overrides.entry(node.to_owned()).or_default()
    }

    /// Set (or clear with `None`) a node's manual cell.
    pub fn set_cell(&mut self, node: &str, cell: Option<[i64; 2]>) {
        self.entry(node).cell = cell;
        self.prune(node);
    }

    /// Set (or clear with `None`) a node's preview override.
    pub fn set_preview(&mut self, node: &str, preview: Option<bool>) {
        self.entry(node).preview = preview;
        self.prune(node);
    }

    /// Set (or clear with `None`) a node's collapsed rendering (wave 4 B4:
    /// the collapsed slider — docs/16 §Canvas conventions). Expanded is the
    /// default, so an override of `false` is no override: callers pass
    /// `None` for it and the entry is pruned (near-empty by construction).
    pub fn set_collapsed(&mut self, node: &str, collapsed: Option<bool>) {
        self.entry(node).collapsed = collapsed;
        self.prune(node);
    }

    /// Rename a node's key (rename is atomic across text + sidecar,
    /// docs/10). A missing entry is a no-op.
    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(entry) = self.overrides.remove(old) {
            self.overrides.insert(new.to_owned(), entry);
        }
        for group in &mut self.groups {
            if let Some(members) = group
                .get_mut("members")
                .and_then(serde_json::Value::as_array_mut)
            {
                for member in members {
                    if member.as_str() == Some(old) {
                        *member = serde_json::Value::String(new.to_owned());
                    }
                }
            }
        }
    }

    /// Drop a node's overrides (on delete).
    pub fn remove(&mut self, node: &str) {
        self.overrides.remove(node);
    }

    /// Drop a node from every group's `members` (on delete) — a deleted
    /// binding must not linger as a phantom member.
    pub fn remove_from_groups(&mut self, node: &str) {
        for group in &mut self.groups {
            if let Some(members) = group
                .get_mut("members")
                .and_then(serde_json::Value::as_array_mut)
            {
                members.retain(|member| member.as_str() != Some(node));
            }
        }
    }

    fn prune(&mut self, node: &str) {
        if self.overrides.get(node).is_some_and(Override::is_empty) {
            self.overrides.remove(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_name_dot_layout_json() {
        assert_eq!(
            Sidecar::path_for(Path::new("a/b/wall.cic")),
            PathBuf::from("a/b/wall.cic.layout.json")
        );
    }

    #[test]
    fn unknown_keys_survive_a_round_trip() {
        let text = r##"{
  "version": 1,
  "overrides": {
    "carved": { "cell": [34, 7], "color": "#7a5c3f", "collapsed": true,
                "port_order": ["cutter", "solid"], "preview": false, "future": {"x": 1} }
  },
  "groups": [ { "title": "Carve stage", "members": ["cutters", "carved"], "collapsed": false } ],
  "views": { "bookmarks": [] },
  "someday": true
}"##;
        let sidecar = Sidecar::parse(text).unwrap();
        assert_eq!(sidecar.overrides["carved"].cell, Some([34, 7]));
        assert_eq!(sidecar.overrides["carved"].extra["future"]["x"], 1);
        assert_eq!(sidecar.extra["someday"], true);
        let again = Sidecar::parse(&sidecar.render()).unwrap();
        assert_eq!(again, sidecar);
    }

    #[test]
    fn empty_state_means_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.cic.layout.json");
        let mut sidecar = Sidecar::default();
        sidecar.set_cell("a", Some([1, 2]));
        sidecar.save(&path).unwrap();
        assert!(path.exists());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            sidecar.render(),
            "the file holds exactly the rendered bytes"
        );
        assert!(
            !dir.path().join(".p.cic.layout.json.cicada-tmp").exists(),
            "the atomic write leaves no temp file"
        );
        sidecar.set_cell("a", None);
        assert!(sidecar.overrides.is_empty(), "empty entries are pruned");
        sidecar.save(&path).unwrap();
        assert!(!path.exists(), "no state → no file");
    }

    // Wave 4 B4: the collapsed flag rides the same entry as the cell, and
    // clearing it prunes the entry like every other override.
    #[test]
    fn collapsed_is_an_override_that_prunes_when_cleared() {
        let mut sidecar = Sidecar::default();
        sidecar.set_collapsed("size", Some(true));
        assert_eq!(sidecar.overrides["size"].collapsed, Some(true));
        assert!(sidecar.render().contains("\"collapsed\": true"));
        sidecar.set_cell("size", Some([3, 4]));
        sidecar.set_collapsed("size", None);
        assert_eq!(
            sidecar.overrides["size"],
            Override {
                cell: Some([3, 4]),
                ..Override::default()
            },
            "the cell survives the flag's clearing"
        );
        sidecar.set_cell("size", None);
        assert!(sidecar.overrides.is_empty(), "no overrides → no entry");
    }

    #[test]
    fn rename_moves_key_and_group_membership() {
        let mut sidecar = Sidecar::default();
        sidecar.set_cell("old", Some([0, 0]));
        sidecar
            .groups
            .push(serde_json::json!({"title": "g", "members": ["old", "z"]}));
        sidecar.rename("old", "new");
        assert!(sidecar.overrides.contains_key("new"));
        assert!(!sidecar.overrides.contains_key("old"));
        assert_eq!(sidecar.groups[0]["members"][0], "new");
    }

    #[test]
    fn newer_version_is_refused() {
        assert!(Sidecar::parse(r#"{"version": 99}"#).is_err());
        assert!(Sidecar::parse("nonsense").is_err());
    }
}
