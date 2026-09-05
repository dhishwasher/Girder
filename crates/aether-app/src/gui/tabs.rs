use crate::app::{active_python_path, AetherApp};
use crate::project::{ProjectWorkspace, SyncImpact};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

const STATE_PATH: &str = ".girder/ui-state.json";

#[derive(Debug, Clone)]
struct EditorTab {
    path: String,
    dirty: bool,
    preview: bool,
    draft: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedTabs {
    open: Vec<String>,
    active: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TabAction {
    Activate(String),
    Close(String),
    CloseCurrent,
    Next,
    Previous,
}

pub(crate) struct EditorTabs {
    root: PathBuf,
    tabs: Vec<EditorTab>,
    active: Option<usize>,
    persistence_error: Option<String>,
}

pub(crate) struct FileSwitchOutcome {
    pub(crate) impacts: Vec<SyncImpact>,
    pub(crate) error: Option<std::io::Error>,
}

impl EditorTabs {
    pub(crate) fn for_workspace(workspace: &mut ProjectWorkspace) -> Self {
        let root = workspace.root().to_path_buf();
        let initial = workspace.active_file().map(str::to_string);
        let tabs = Self::load(&root, initial.as_deref());
        let Some(active) = tabs.active_path() else {
            return tabs;
        };
        if workspace.active_file() == Some(active) || workspace.select_file(active).is_ok() {
            tabs
        } else {
            Self::fresh(root, initial.as_deref())
        }
    }

    pub(crate) fn load(root: impl Into<PathBuf>, initial: Option<&str>) -> Self {
        let root = root.into();
        let state_path = root.join(STATE_PATH);
        let (persisted, persistence_error) = match fs::read(&state_path) {
            Ok(bytes) => match serde_json::from_slice::<PersistedTabs>(&bytes) {
                Ok(state) => (state, None),
                Err(error) => (
                    PersistedTabs::default(),
                    Some(format!("could not read {}: {error}", state_path.display())),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (PersistedTabs::default(), None)
            }
            Err(error) => (
                PersistedTabs::default(),
                Some(format!("could not read {}: {error}", state_path.display())),
            ),
        };

        let mut tabs: Vec<EditorTab> = persisted
            .open
            .into_iter()
            .filter(|path| safe_relative(path) && root.join(path).is_file())
            .map(|path| EditorTab {
                path,
                dirty: false,
                preview: false,
                draft: None,
            })
            .collect();
        let mut seen = HashSet::new();
        tabs.retain(|tab| seen.insert(tab.path.clone()));

        let active = persisted.active.filter(|index| *index < tabs.len());
        if tabs.is_empty() {
            if let Some(initial) = initial.filter(|path| safe_relative(path)) {
                tabs.push(EditorTab {
                    path: initial.to_string(),
                    dirty: false,
                    preview: false,
                    draft: None,
                });
            }
        }
        let active = active.or_else(|| (!tabs.is_empty()).then_some(0));

        Self {
            root,
            tabs,
            active,
            persistence_error,
        }
    }

    fn fresh(root: PathBuf, initial: Option<&str>) -> Self {
        let tabs = initial
            .filter(|path| safe_relative(path))
            .map(|path| {
                vec![EditorTab {
                    path: path.to_string(),
                    dirty: false,
                    preview: false,
                    draft: None,
                }]
            })
            .unwrap_or_default();
        let active = (!tabs.is_empty()).then_some(0);
        Self {
            root,
            tabs,
            active,
            persistence_error: None,
        }
    }

    pub(crate) fn active_path(&self) -> Option<&str> {
        self.active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.path.as_str())
    }

    pub(crate) fn open(&mut self, path: &str, preview: bool) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            self.active = Some(index);
            if !preview {
                self.tabs[index].preview = false;
            }
            self.persist();
            return;
        }

        let new_tab = EditorTab {
            path: path.to_string(),
            dirty: false,
            preview,
            draft: None,
        };
        if preview {
            if let Some(index) = self.tabs.iter().position(|tab| tab.preview && !tab.dirty) {
                self.tabs[index] = new_tab;
                self.active = Some(index);
                self.persist();
                return;
            }
        }
        self.tabs.push(new_tab);
        self.active = Some(self.tabs.len() - 1);
        self.persist();
    }

    pub(crate) fn capture(&mut self, path: &str, source: &str, dirty: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path) {
            tab.dirty = dirty;
            tab.draft = dirty.then(|| source.to_string());
            if dirty {
                tab.preview = false;
            }
        }
    }

    pub(crate) fn draft(&self, path: &str) -> Option<String> {
        self.tabs
            .iter()
            .find(|tab| tab.path == path)
            .and_then(|tab| tab.draft.clone())
    }

    pub(crate) fn set_dirty(&mut self, path: &str, dirty: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.path == path) {
            tab.dirty = dirty;
            if dirty {
                tab.preview = false;
            } else {
                tab.draft = None;
            }
        }
    }

    pub(crate) fn is_dirty(&self, path: &str) -> bool {
        self.tabs
            .iter()
            .find(|tab| tab.path == path)
            .is_some_and(|tab| tab.dirty)
    }

    pub(crate) fn has_dirty(&self) -> bool {
        self.tabs.iter().any(|tab| tab.dirty)
    }

    pub(crate) fn is_preview(&self, path: &str) -> bool {
        self.tabs
            .iter()
            .find(|tab| tab.path == path)
            .is_some_and(|tab| tab.preview)
    }

    pub(crate) fn remove(&mut self, path: &str) {
        let Some(index) = self.tabs.iter().position(|tab| tab.path == path) else {
            return;
        };
        let active_path = self.active_path().map(str::to_string);
        self.tabs.remove(index);
        self.active = match active_path {
            Some(active) if active != path => self.tabs.iter().position(|tab| tab.path == active),
            _ if self.tabs.is_empty() => None,
            _ => Some(index.min(self.tabs.len() - 1)),
        };
        self.persist();
    }

    pub(crate) fn next_path(&self, direction: isize) -> Option<String> {
        if self.tabs.is_empty() {
            return None;
        }
        let current = self.active.unwrap_or(0) as isize;
        let len = self.tabs.len() as isize;
        let next = (current + direction).rem_euclid(len) as usize;
        Some(self.tabs[next].path.clone())
    }

    pub(crate) fn persistence_error(&self) -> Option<&str> {
        self.persistence_error.as_deref()
    }

    fn persist(&mut self) {
        let state_path = self.root.join(STATE_PATH);
        let Some(parent) = state_path.parent() else {
            return;
        };
        let result = (|| -> std::io::Result<()> {
            fs::create_dir_all(parent)?;
            let persisted = PersistedTabs {
                open: self.tabs.iter().map(|tab| tab.path.clone()).collect(),
                active: self.active,
            };
            let bytes = serde_json::to_vec_pretty(&persisted)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            fs::write(&state_path, bytes)
        })();
        self.persistence_error = result
            .err()
            .map(|error| format!("could not save {}: {error}", state_path.display()));
    }
}

pub(crate) fn switch_workspace_file(
    workspace: &mut ProjectWorkspace,
    tabs: &mut EditorTabs,
    path: &str,
    preview: bool,
) -> FileSwitchOutcome {
    let mut impacts = Vec::new();
    if workspace.active_file() == Some(path) {
        tabs.open(path, preview);
        tabs.set_dirty(path, workspace.is_dirty());
        return FileSwitchOutcome {
            impacts,
            error: None,
        };
    }

    let previous = workspace.active_file().map(str::to_string);
    let previous_dirty = workspace.is_dirty();
    let previous_source = previous_dirty.then(|| workspace.buffer().to_string());
    if let Some(previous) = &previous {
        tabs.capture(previous, workspace.buffer(), previous_dirty);
    }
    if previous_dirty {
        match workspace.discard_changes() {
            Ok(impact) => impacts.push(impact),
            Err(error) => {
                return FileSwitchOutcome {
                    impacts,
                    error: Some(error),
                };
            }
        }
    }

    let target_draft = tabs.draft(path);
    if let Err(error) = workspace.select_file(path) {
        if let Some(source) = previous_source {
            workspace.buffer_mut().clone_from(&source);
            if let Ok(impact) = workspace.sync_buffer_to_graph() {
                impacts.push(impact);
            }
        }
        return FileSwitchOutcome {
            impacts,
            error: Some(error),
        };
    }

    tabs.open(path, preview);
    if let Some(source) = target_draft {
        workspace.buffer_mut().clone_from(&source);
        match workspace.sync_buffer_to_graph() {
            Ok(impact) => impacts.push(impact),
            Err(error) => {
                return FileSwitchOutcome {
                    impacts,
                    error: Some(error),
                };
            }
        }
    }
    tabs.set_dirty(path, workspace.is_dirty());
    FileSwitchOutcome {
        impacts,
        error: None,
    }
}

impl AetherApp {
    pub(crate) fn select_file(&mut self, relative: &str) {
        self.activate_editor_file(relative, false);
    }

    pub(crate) fn preview_file(&mut self, relative: &str) {
        self.activate_editor_file(relative, true);
    }

    fn activate_editor_file(&mut self, relative: &str, preview: bool) {
        if self.extension_busy() {
            self.set_workspace_error("Wait for the extension operation before opening a file.");
            return;
        }
        let outcome = switch_workspace_file(
            &mut self.workspace,
            &mut self.editor_tabs,
            relative,
            preview,
        );
        for impact in outcome.impacts {
            self.apply_sync_impact(impact);
        }
        if let Some(error) = outcome.error {
            self.set_workspace_error(format!("File switch failed: {error}"));
            return;
        }

        self.py_file = active_python_path(&self.workspace).unwrap_or_default();
        self.py_steps.clear();
        let verb = if preview { "Previewing" } else { "Opened" };
        self.set_workspace_status(format!("{verb} {relative}"));
    }

    pub(crate) fn handle_tab_action(&mut self, action: TabAction) {
        match action {
            TabAction::Activate(path) => {
                let preview = self.editor_tabs.is_preview(&path);
                self.activate_editor_file(&path, preview);
            }
            TabAction::Close(path) => self.close_editor_tab(&path),
            TabAction::CloseCurrent => {
                if let Some(path) = self.editor_tabs.active_path().map(str::to_string) {
                    self.close_editor_tab(&path);
                }
            }
            TabAction::Next => {
                if let Some(path) = self.editor_tabs.next_path(1) {
                    let preview = self.editor_tabs.is_preview(&path);
                    self.activate_editor_file(&path, preview);
                }
            }
            TabAction::Previous => {
                if let Some(path) = self.editor_tabs.next_path(-1) {
                    let preview = self.editor_tabs.is_preview(&path);
                    self.activate_editor_file(&path, preview);
                }
            }
        }
    }

    fn close_editor_tab(&mut self, path: &str) {
        if self.workspace.active_file() == Some(path) {
            self.editor_tabs.set_dirty(path, self.workspace.is_dirty());
        }
        if self.editor_tabs.is_dirty(path) {
            self.set_workspace_error(format!(
                "Save or discard the changes in {path} before closing its tab."
            ));
            return;
        }

        let was_active = self.editor_tabs.active_path() == Some(path);
        self.editor_tabs.remove(path);
        if was_active {
            if let Some(next) = self.editor_tabs.active_path().map(str::to_string) {
                let preview = self.editor_tabs.is_preview(&next);
                self.activate_editor_file(&next, preview);
            } else {
                self.set_workspace_status(format!("Closed {path}"));
            }
        }
    }
}

pub(crate) fn show(state: &mut EditorTabs, ui: &mut egui::Ui) -> Option<TabAction> {
    let active = state.active_path().map(str::to_string);
    let tabs: Vec<_> = state
        .tabs
        .iter()
        .map(|tab| (tab.path.clone(), tab.dirty, tab.preview))
        .collect();
    let mut action = None;

    egui::ScrollArea::horizontal()
        .id_salt("editor_tabs")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (path, dirty, preview) in tabs {
                    let name = Path::new(&path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&path);
                    let mut text = egui::RichText::new(if dirty {
                        format!("{name} ●")
                    } else {
                        name.to_string()
                    });
                    if preview {
                        text = text.italics();
                    }
                    let response = ui
                        .selectable_label(active.as_deref() == Some(path.as_str()), text)
                        .on_hover_text(&path);
                    if response.clicked_by(egui::PointerButton::Middle) {
                        action = Some(TabAction::Close(path.clone()));
                    } else if response.clicked() {
                        action = Some(TabAction::Activate(path.clone()));
                    }
                    if ui.small_button("×").on_hover_text("Close tab").clicked() {
                        action = Some(TabAction::Close(path));
                    }
                }
            });
        });

    if let Some(error) = state.persistence_error() {
        ui.colored_label(crate::gui::theme::PALETTE.error, error);
    }
    action
}

pub(crate) fn keyboard_action(ctx: &egui::Context) -> Option<TabAction> {
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::W)) {
        return Some(TabAction::CloseCurrent);
    }

    let mut reverse = egui::Modifiers::CTRL;
    reverse.shift = true;
    if ctx.input_mut(|input| input.consume_key(reverse, egui::Key::Tab)) {
        return Some(TabAction::Previous);
    }
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::Tab)) {
        return Some(TabAction::Next);
    }
    None
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).is_relative()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("girder-tabs-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).expect("create test workspace");
        root
    }

    #[test]
    fn preview_is_reused_until_opened_or_edited() {
        let root = workspace();
        let mut tabs = EditorTabs::load(&root, None);
        tabs.open("one.rs", true);
        tabs.open("two.rs", true);
        assert_eq!(tabs.tabs.len(), 1);
        assert_eq!(tabs.active_path(), Some("two.rs"));

        tabs.open("two.rs", false);
        tabs.open("three.rs", true);
        tabs.set_dirty("three.rs", true);
        tabs.open("four.rs", true);
        assert_eq!(tabs.tabs.len(), 3);
        assert!(!tabs.is_preview("three.rs"));

        fs::remove_dir_all(root).expect("remove test workspace");
    }

    #[test]
    fn open_tabs_and_active_index_round_trip() {
        let root = workspace();
        fs::write(root.join("one.rs"), "fn one() {}\n").expect("write source");
        fs::write(root.join("two.rs"), "fn two() {}\n").expect("write source");
        let mut tabs = EditorTabs::load(&root, Some("one.rs"));
        tabs.open("two.rs", false);

        let restored = EditorTabs::load(&root, None);
        assert_eq!(restored.tabs.len(), 2);
        assert_eq!(restored.active_path(), Some("two.rs"));

        fs::remove_dir_all(root).expect("remove test workspace");
    }

    #[test]
    fn unsafe_persisted_paths_are_rejected() {
        assert!(!safe_relative("../outside.rs"));
        assert!(!safe_relative("/absolute.rs"));
        assert!(safe_relative("src/lib.rs"));
    }
}
