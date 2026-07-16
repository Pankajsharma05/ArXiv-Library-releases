use crate::db::data_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// User's display name. None until first-run prompt is completed.
    pub name: Option<String>,
    /// ISO date the app was first run.
    pub installed_at: Option<String>,
    /// Map of "YYYY-MM-DD" -> seconds spent that day.
    #[serde(default)]
    pub usage: BTreeMap<String, u64>,
    /// arXiv categories the user follows for the daily feed.
    #[serde(default)]
    pub followed_categories: Vec<String>,
    /// Whether view-history tracking is enabled (default off).
    #[serde(default)]
    pub history_enabled: bool,
    /// User-defined saved searches pinned in the sidebar.
    #[serde(default)]
    pub saved_searches: Vec<SavedSearch>,
    /// UI font scale in percent (e.g. 100 = default). Clamped client-side.
    #[serde(default = "default_font_scale")]
    pub font_scale: u32,
    /// UI font family key: "system", "serif", "mono", or "rounded".
    #[serde(default = "default_font_family")]
    pub font_family: String,
}

fn default_font_scale() -> u32 { 100 }
fn default_font_family() -> String { "system".to_string() }

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: None,
            installed_at: None,
            usage: BTreeMap::new(),
            followed_categories: Vec::new(),
            history_enabled: false,
            saved_searches: Vec::new(),
            font_scale: 100,
            font_family: "system".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSearch {
    pub id: String,
    pub title: String,
    pub keywords: String,
    pub field: String,
    pub sort: String,
}

pub struct SettingsStore {
    inner: Mutex<Settings>,
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

impl SettingsStore {
    pub fn load() -> Self {
        let s = std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
            .unwrap_or_default();
        SettingsStore { inner: Mutex::new(s) }
    }

    fn persist(&self, s: &Settings) {
        if let Ok(json) = serde_json::to_string_pretty(s) {
            let _ = std::fs::write(settings_path(), json);
        }
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().unwrap().clone()
    }

    pub fn set_name(&self, name: String) {
        let mut s = self.inner.lock().unwrap();
        s.name = Some(name);
        if s.installed_at.is_none() {
            s.installed_at = Some(today());
        }
        self.persist(&s);
    }

    /// Add `seconds` to today's usage bucket.
    pub fn add_usage(&self, seconds: u64) {
        let mut s = self.inner.lock().unwrap();
        if s.installed_at.is_none() {
            s.installed_at = Some(today());
        }
        *s.usage.entry(today()).or_insert(0) += seconds;
        self.persist(&s);
    }

    pub fn set_followed_categories(&self, cats: Vec<String>) {
        let mut s = self.inner.lock().unwrap();
        s.followed_categories = cats;
        self.persist(&s);
    }

    pub fn set_history_enabled(&self, enabled: bool) {
        let mut s = self.inner.lock().unwrap();
        s.history_enabled = enabled;
        self.persist(&s);
    }

    pub fn set_saved_searches(&self, searches: Vec<SavedSearch>) {
        let mut s = self.inner.lock().unwrap();
        s.saved_searches = searches;
        self.persist(&s);
    }

    pub fn set_appearance(&self, font_scale: u32, font_family: String) {
        let mut s = self.inner.lock().unwrap();
        s.font_scale = font_scale.clamp(70, 160);
        s.font_family = font_family;
        self.persist(&s);
    }

    /// Replace name + followed categories from an imported settings blob
    /// (used by settings export/import). Usage history is merged, not replaced.
    pub fn import_from(&self, other: &Settings) {
        let mut s = self.inner.lock().unwrap();
        if other.name.is_some() { s.name = other.name.clone(); }
        if !other.followed_categories.is_empty() {
            s.followed_categories = other.followed_categories.clone();
        }
        for (k, v) in &other.usage {
            let e = s.usage.entry(k.clone()).or_insert(0);
            *e = (*e).max(*v); // keep the larger of the two for any given day
        }
        self.persist(&s);
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
