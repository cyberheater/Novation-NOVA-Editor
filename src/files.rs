//! File dialogs and the settings folder, per platform.
//!
//! Desktop: the native dialogs from `rfd`. iPadOS: saves go straight into the app's Documents
//! folder (visible in the Files app under "On My iPad > Nova Editor"); opening files is not
//! wired up yet (needs the system document picker), so the pick functions return None.
use std::path::PathBuf;

#[cfg(not(target_os = "ios"))]
pub use rfd::FileDialog;

/// Folder for the app's own settings files.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "ios")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    }
    #[cfg(not(target_os = "ios"))]
    {
        dirs::config_dir()
    }
}

/// iPadOS: the app's Documents folder.
#[cfg(target_os = "ios")]
pub fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Documents"))
}

/// Same calls as `rfd::FileDialog`, for iPadOS.
#[cfg(target_os = "ios")]
#[derive(Default)]
pub struct FileDialog {
    name: Option<String>,
}

#[cfg(target_os = "ios")]
impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    pub fn add_filter(self, _name: &str, _ext: &[&str]) -> Self {
        self
    }
    /// The Documents path for the suggested name (a dialog is not shown).
    pub fn save_file(self) -> Option<PathBuf> {
        let dir = documents_dir()?;
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join(self.name.unwrap_or_else(|| "nova.syx".into())))
    }
    pub fn pick_file(self) -> Option<PathBuf> {
        None
    }
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        None
    }
}
