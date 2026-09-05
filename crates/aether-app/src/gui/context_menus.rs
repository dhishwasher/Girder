use crate::app::AetherApp;
use crate::graph_view::GraphScope;
use aether_graph::NodeId;
use egui::text::{CCursor, CCursorRange};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorMenuAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
    GoToDefinition,
    FindCallers,
    ShowImpact,
    CopySemanticPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileMenuAction {
    Open { path: String, is_dir: bool },
    Rename { path: String, is_dir: bool },
    Delete { path: String, is_dir: bool },
    CopyPath(String),
    Reveal(String),
}

enum FileDialog {
    Rename {
        relative: String,
        new_name: String,
        is_dir: bool,
    },
    Delete {
        relative: String,
        is_dir: bool,
    },
}

enum ConfirmedFileOperation {
    Rename { from: String, to: String },
    Delete { relative: String, is_dir: bool },
}

#[derive(Default)]
pub(crate) struct ContextMenuState {
    dialog: Option<FileDialog>,
}

impl ContextMenuState {
    fn request_rename(&mut self, relative: String, is_dir: bool) {
        let new_name = Path::new(&relative)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        self.dialog = Some(FileDialog::Rename {
            relative,
            new_name,
            is_dir,
        });
    }

    fn request_delete(&mut self, relative: String, is_dir: bool) {
        self.dialog = Some(FileDialog::Delete { relative, is_dir });
    }

    fn show_dialog(&mut self, ctx: &egui::Context) -> Option<ConfirmedFileOperation> {
        let mut dialog = self.dialog.take()?;
        let mut keep_open = true;
        let mut confirmed = None;

        match &mut dialog {
            FileDialog::Rename {
                relative,
                new_name,
                is_dir,
            } => {
                let mut window_open = true;
                egui::Window::new("Rename")
                    .collapsible(false)
                    .resizable(false)
                    .open(&mut window_open)
                    .show(ctx, |ui| {
                        ui.label(relative.as_str());
                        let response = ui.add(
                            egui::TextEdit::singleline(new_name)
                                .desired_width(320.0)
                                .hint_text("New name"),
                        );
                        response.request_focus();
                        let valid = valid_file_name(new_name);
                        if !valid {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                "Use one non-empty file or directory name.",
                            );
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                keep_open = false;
                            }
                            if ui.add_enabled(valid, egui::Button::new("Rename")).clicked() {
                                let parent = Path::new(relative).parent().unwrap_or(Path::new(""));
                                let destination = parent.join(new_name.as_str());
                                confirmed = Some(ConfirmedFileOperation::Rename {
                                    from: relative.clone(),
                                    to: normalized_relative(&destination),
                                });
                                keep_open = false;
                            }
                        });
                        if *is_dir {
                            ui.weak("Every open path under this directory will be reloaded.");
                        }
                    });
                keep_open &= window_open;
            }
            FileDialog::Delete { relative, is_dir } => {
                let mut window_open = true;
                egui::Window::new("Confirm delete")
                    .collapsible(false)
                    .resizable(false)
                    .open(&mut window_open)
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Delete {} `{relative}` from disk?",
                            if *is_dir { "directory" } else { "file" }
                        ));
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            "This cannot be undone by Girder.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                keep_open = false;
                            }
                            if ui
                                .add(egui::Button::new("Delete").fill(egui::Color32::DARK_RED))
                                .clicked()
                            {
                                confirmed = Some(ConfirmedFileOperation::Delete {
                                    relative: relative.clone(),
                                    is_dir: *is_dir,
                                });
                                keep_open = false;
                            }
                        });
                    });
                keep_open &= window_open;
            }
        }

        if keep_open {
            self.dialog = Some(dialog);
        }
        confirmed
    }
}

pub(crate) fn editor_menu(
    response: &egui::Response,
    has_selection: bool,
) -> Option<EditorMenuAction> {
    let mut action = None;
    response.context_menu(|ui| {
        if ui
            .add_enabled(has_selection, egui::Button::new("Cut"))
            .clicked()
        {
            action = Some(EditorMenuAction::Cut);
            ui.close_menu();
        }
        if ui
            .add_enabled(has_selection, egui::Button::new("Copy"))
            .clicked()
        {
            action = Some(EditorMenuAction::Copy);
            ui.close_menu();
        }
        if ui.button("Paste").clicked() {
            action = Some(EditorMenuAction::Paste);
            ui.close_menu();
        }
        if ui.button("Select all").clicked() {
            action = Some(EditorMenuAction::SelectAll);
            ui.close_menu();
        }
        ui.separator();
        for (label, candidate) in [
            ("Go to Definition", EditorMenuAction::GoToDefinition),
            ("Find Callers", EditorMenuAction::FindCallers),
            ("Show Impact", EditorMenuAction::ShowImpact),
            ("Copy Semantic Path", EditorMenuAction::CopySemanticPath),
        ] {
            if ui.button(label).clicked() {
                action = Some(candidate);
                ui.close_menu();
            }
        }
    });
    action
}

pub(crate) fn file_entry_menu(
    response: &egui::Response,
    path: &str,
    is_dir: bool,
) -> Option<FileMenuAction> {
    let mut action = None;
    response.context_menu(|ui| {
        let mut choose = |ui: &mut egui::Ui, label: &str, candidate: FileMenuAction| {
            if ui.button(label).clicked() {
                action = Some(candidate);
                ui.close_menu();
            }
        };
        choose(
            ui,
            "Open",
            FileMenuAction::Open {
                path: path.to_string(),
                is_dir,
            },
        );
        choose(
            ui,
            "Rename…",
            FileMenuAction::Rename {
                path: path.to_string(),
                is_dir,
            },
        );
        choose(
            ui,
            "Delete…",
            FileMenuAction::Delete {
                path: path.to_string(),
                is_dir,
            },
        );
        ui.separator();
        choose(ui, "Copy path", FileMenuAction::CopyPath(path.to_string()));
        choose(
            ui,
            "Reveal in file manager",
            FileMenuAction::Reveal(path.to_string()),
        );
    });
    action
}

pub(crate) fn apply_text_action(
    action: EditorMenuAction,
    output: &mut egui::text_edit::TextEditOutput,
    text: &mut String,
    ctx: &egui::Context,
) -> Result<bool, String> {
    let char_count = text.chars().count();
    let range = output
        .state
        .cursor
        .char_range()
        .unwrap_or_else(|| CCursorRange::one(CCursor::new(char_count)));
    let [min, max] = range.sorted();
    let start = char_to_byte(text, min.index);
    let end = char_to_byte(text, max.index);
    let mut changed = false;

    match action {
        EditorMenuAction::Copy | EditorMenuAction::Cut if start != end => {
            ctx.copy_text(text[start..end].to_string());
            if action == EditorMenuAction::Cut {
                text.replace_range(start..end, "");
                output
                    .state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(min)));
                changed = true;
            }
        }
        EditorMenuAction::Paste => {
            let clipboard = arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.get_text())
                .map_err(|error| format!("Clipboard paste failed: {error}"))?;
            if !clipboard.is_empty() {
                text.replace_range(start..end, &clipboard);
                let cursor = CCursor::new(min.index + clipboard.chars().count());
                output
                    .state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(cursor)));
                changed = true;
            }
        }
        EditorMenuAction::SelectAll => {
            output.state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(0),
                CCursor::new(char_count),
            )));
        }
        _ => {}
    }

    output.state.clone().store(ctx, output.response.id);
    output.response.request_focus();
    Ok(changed)
}

impl AetherApp {
    pub(crate) fn handle_editor_semantic_action(
        &mut self,
        action: EditorMenuAction,
        cursor_char: usize,
        ctx: &egui::Context,
    ) {
        let Some((node_id, path)) = semantic_node_at_cursor(self, cursor_char) else {
            self.set_workspace_error("No semantic node was found at the editor cursor.");
            return;
        };

        match action {
            EditorMenuAction::GoToDefinition => self.navigate_to_graph_node(node_id),
            EditorMenuAction::CopySemanticPath => {
                ctx.copy_text(path.clone());
                self.set_workspace_status(format!("Copied semantic path {path}"));
            }
            EditorMenuAction::FindCallers => {
                let callers = {
                    let graph = self.workspace.graph().lock().unwrap();
                    graph
                        .callers(node_id)
                        .into_iter()
                        .filter_map(|caller| {
                            graph
                                .get(caller.id)
                                .map(|node| (caller.id, node.path.clone()))
                        })
                        .collect::<Vec<_>>()
                };
                self.impact_nodes = callers
                    .iter()
                    .map(|(caller, _)| (*caller, 1))
                    .chain(std::iter::once((node_id, 0)))
                    .collect();
                self.ripple_start = Some(Instant::now());
                self.focus_graph_node(node_id);
                let names: Vec<_> = callers
                    .iter()
                    .take(5)
                    .map(|(_, path)| path.as_str())
                    .collect();
                self.set_workspace_status(if names.is_empty() {
                    format!("{path} has no recorded callers.")
                } else {
                    format!(
                        "{} caller(s) of {path}: {}",
                        callers.len(),
                        names.join(", ")
                    )
                });
            }
            EditorMenuAction::ShowImpact => {
                let affected = {
                    let graph = self.workspace.graph().lock().unwrap();
                    graph.impact_of(node_id).affected
                };
                let count = affected.len();
                self.impact_nodes = affected;
                self.impact_nodes.insert(node_id, 0);
                self.ripple_start = Some(Instant::now());
                self.focus_graph_node(node_id);
                self.set_workspace_status(format!(
                    "{path} impacts {count} downstream semantic node(s)."
                ));
            }
            _ => {}
        }
    }

    pub(crate) fn handle_file_menu_action(&mut self, action: FileMenuAction, ctx: &egui::Context) {
        match action {
            FileMenuAction::Open {
                path,
                is_dir: false,
            } => self.select_file(&path),
            FileMenuAction::Open { .. } => {}
            FileMenuAction::Rename { path, is_dir } => {
                self.context_menus.request_rename(path, is_dir);
            }
            FileMenuAction::Delete { path, is_dir } => {
                self.context_menus.request_delete(path, is_dir);
            }
            FileMenuAction::CopyPath(relative) => {
                let path = self.workspace.root().join(relative);
                ctx.copy_text(path.display().to_string());
                self.set_workspace_status(format!("Copied {}", path.display()));
            }
            FileMenuAction::Reveal(relative) => {
                let path = self.workspace.root().join(relative);
                match reveal_in_file_manager(&path) {
                    Ok(()) => self.set_workspace_status(format!("Revealed {}", path.display())),
                    Err(error) => self.set_workspace_error(format!("Reveal failed: {error}")),
                }
            }
        }
    }

    pub(crate) fn show_context_dialogs(&mut self, ctx: &egui::Context) {
        let Some(operation) = self.context_menus.show_dialog(ctx) else {
            return;
        };
        if self.workspace.is_dirty() || self.editor_tabs.has_dirty() {
            self.set_workspace_error(
                "Save or discard every modified tab before renaming or deleting files.",
            );
            return;
        }

        let root = self.workspace.root().to_path_buf();
        let result = match operation {
            ConfirmedFileOperation::Rename { from, to } => {
                rename_entry(&root, &from, &to).map(|()| format!("Renamed {from} to {to}"))
            }
            ConfirmedFileOperation::Delete { relative, is_dir } => {
                delete_entry(&root, &relative, is_dir).map(|()| format!("Deleted {relative}"))
            }
        };
        match result {
            Ok(message) => {
                self.open_project();
                if !self.workspace_status_is_error {
                    self.set_workspace_status(message);
                }
            }
            Err(error) => self.set_workspace_error(error.to_string()),
        }
    }

    fn focus_graph_node(&mut self, node_id: NodeId) {
        self.graph_view.selected = Some(node_id);
        self.graph_view.scope = GraphScope::OneHop;
        self.graph_view.request_fit();
    }
}

fn semantic_node_at_cursor(app: &AetherApp, cursor_char: usize) -> Option<(NodeId, String)> {
    let active_file = app.workspace.active_file()?;
    let source = app.workspace.buffer();
    let identifier = identifier_at(source, cursor_char);
    let cursor_byte = char_to_byte(source, cursor_char);
    let graph = app.workspace.graph().lock().ok()?;

    if let Some(identifier) = identifier {
        let mut exact: Vec<_> = graph
            .nodes()
            .filter(|node| node.name == identifier && node.file.is_some())
            .map(|node| {
                (
                    node.file.as_deref() != Some(active_file),
                    node.path.clone(),
                    node.id,
                )
            })
            .collect();
        exact.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
        if let Some((_, path, id)) = exact.into_iter().next() {
            return Some((id, path));
        }
    }

    graph
        .nodes()
        .filter(|node| {
            node.file.as_deref() == Some(active_file)
                && node.span.start_byte <= cursor_byte
                && cursor_byte <= node.span.end_byte
        })
        .min_by_key(|node| node.span.end_byte.saturating_sub(node.span.start_byte))
        .map(|node| (node.id, node.path.clone()))
}

fn identifier_at(source: &str, cursor_char: usize) -> Option<String> {
    let chars: Vec<_> = source.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let mut cursor = cursor_char.min(chars.len().saturating_sub(1));
    if !identifier_char(chars[cursor]) && cursor > 0 && identifier_char(chars[cursor - 1]) {
        cursor -= 1;
    }
    if !identifier_char(chars[cursor]) {
        return None;
    }
    let mut start = cursor;
    while start > 0 && identifier_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = cursor + 1;
    while end < chars.len() && identifier_char(chars[end]) {
        end += 1;
    }
    Some(chars[start..end].iter().collect())
}

fn identifier_char(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn checked_relative(root: &Path, relative: &str) -> std::io::Result<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || !path.is_relative()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unsafe workspace path: {relative}"),
        ));
    }
    Ok(root.join(path))
}

fn rename_entry(root: &Path, from: &str, to: &str) -> std::io::Result<()> {
    let from = checked_relative(root, from)?;
    let to = checked_relative(root, to)?;
    if to.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists", to.display()),
        ));
    }
    fs::rename(from, to)
}

fn delete_entry(root: &Path, relative: &str, is_dir: bool) -> std::io::Result<()> {
    let path = checked_relative(root, relative)?;
    let metadata = fs::symlink_metadata(&path)?;
    if is_dir && metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(format!("/select,{}", path.display()));
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        });
        command
    };
    #[cfg(not(any(unix, target_os = "windows")))]
    let mut command = {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "revealing files is unsupported on this platform",
        ));
    };
    command.spawn().map(|_| ())
}

fn normalized_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_tracks_cursor_and_unicode_offsets() {
        let source = "let café_value = call();";
        assert_eq!(identifier_at(source, 7).as_deref(), Some("café_value"));
        assert_eq!(identifier_at(source, 14).as_deref(), Some("café_value"));
        assert_eq!(char_to_byte(source, 8), 9);
    }

    #[test]
    fn workspace_paths_and_rename_names_are_fail_closed() {
        let root = Path::new("/workspace");
        assert!(checked_relative(root, "src/lib.rs").is_ok());
        assert!(checked_relative(root, "../outside").is_err());
        assert!(valid_file_name("renamed.rs"));
        assert!(!valid_file_name("nested/renamed.rs"));
    }

    #[test]
    fn char_to_byte_clamps_to_end() {
        assert_eq!(char_to_byte("é", 0), 0);
        assert_eq!(char_to_byte("é", 1), 2);
        assert_eq!(char_to_byte("é", 50), 2);
    }
}
