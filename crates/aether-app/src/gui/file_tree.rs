use crate::gui::context_menus::{self, FileMenuAction};
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeEntry {
    relative: PathBuf,
    is_dir: bool,
}

#[derive(Debug, Default)]
struct DirectoryState {
    children: Vec<TreeEntry>,
    expanded: bool,
    loaded: bool,
    loading: bool,
}

struct ScanResult {
    relative: PathBuf,
    result: io::Result<Vec<TreeEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileTreeAction {
    Preview(String),
    Open(String),
    Menu(FileMenuAction),
}

pub(crate) struct FileTreeState {
    root: PathBuf,
    directories: BTreeMap<PathBuf, DirectoryState>,
    filter: String,
    previewed: Option<PathBuf>,
    pending_scans: usize,
    last_error: Option<String>,
    scan_tx: Sender<ScanResult>,
    scan_rx: Receiver<ScanResult>,
}

impl FileTreeState {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let (scan_tx, scan_rx) = mpsc::channel();
        let mut directories = BTreeMap::new();
        directories.insert(
            PathBuf::new(),
            DirectoryState {
                expanded: true,
                loading: true,
                ..DirectoryState::default()
            },
        );

        spawn_scan(root.clone(), PathBuf::new(), scan_tx.clone());
        Self {
            root,
            directories,
            filter: String::new(),
            previewed: None,
            pending_scans: 1,
            last_error: None,
            scan_tx,
            scan_rx,
        }
    }

    pub(crate) fn is_indexing(&self) -> bool {
        self.pending_scans > 0
    }

    fn poll_scans(&mut self) {
        while let Ok(message) = self.scan_rx.try_recv() {
            self.pending_scans = self.pending_scans.saturating_sub(1);
            match message.result {
                Ok(children) => {
                    for child in children.iter().filter(|child| child.is_dir) {
                        self.directories.entry(child.relative.clone()).or_default();
                    }
                    if let Some(directory) = self.directories.get_mut(&message.relative) {
                        directory.children = children;
                        directory.loaded = true;
                        directory.loading = false;
                    }
                }
                Err(error) => {
                    if let Some(directory) = self.directories.get_mut(&message.relative) {
                        directory.loading = false;
                    }
                    self.last_error = Some(error.to_string());
                }
            }
        }
    }

    fn toggle_directory(&mut self, relative: &Path) {
        let Some(directory) = self.directories.get_mut(relative) else {
            return;
        };
        directory.expanded = !directory.expanded;
        if !directory.expanded || directory.loaded || directory.loading {
            return;
        }

        directory.loading = true;
        self.pending_scans += 1;
        spawn_scan(
            self.root.clone(),
            relative.to_path_buf(),
            self.scan_tx.clone(),
        );
    }

    fn file_count(&self) -> usize {
        self.directories
            .values()
            .flat_map(|directory| &directory.children)
            .filter(|entry| !entry.is_dir)
            .count()
    }

    fn show_directory(
        &mut self,
        ui: &mut egui::Ui,
        relative: &Path,
        depth: usize,
        active: Option<&str>,
        filter: &str,
    ) -> Option<FileTreeAction> {
        let entries = self
            .directories
            .get(relative)
            .map(|directory| directory.children.clone())
            .unwrap_or_default();
        let mut action = None;

        for entry in entries {
            let normalized = normalized_relative(&entry.relative);
            if !entry.is_dir
                && !filter.is_empty()
                && !normalized.to_ascii_lowercase().contains(filter)
            {
                continue;
            }

            let name = entry
                .relative
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&normalized);

            if entry.is_dir {
                let directory = self.directories.get(&entry.relative);
                let expanded = directory.is_some_and(|directory| directory.expanded);
                let loading = directory.is_some_and(|directory| directory.loading);
                let mut toggle = false;
                let mut menu_action = None;
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 12.0);
                    let marker = if loading {
                        "…"
                    } else if expanded {
                        "▾"
                    } else {
                        "▸"
                    };
                    let response = ui
                        .selectable_label(false, format!("{marker} {name}"))
                        .on_hover_text(&normalized);
                    if response.clicked() {
                        toggle = true;
                    }
                    menu_action = context_menus::file_entry_menu(&response, &normalized, true);
                });
                if let Some(menu) = menu_action {
                    if matches!(menu, FileMenuAction::Open { .. }) {
                        if !expanded {
                            self.toggle_directory(&entry.relative);
                        }
                    } else {
                        action = Some(FileTreeAction::Menu(menu));
                    }
                }
                if toggle {
                    self.toggle_directory(&entry.relative);
                }
                if self
                    .directories
                    .get(&entry.relative)
                    .is_some_and(|directory| directory.expanded)
                {
                    action = action.or_else(|| {
                        self.show_directory(ui, &entry.relative, depth + 1, active, filter)
                    });
                }
                continue;
            }

            let is_preview = self.previewed.as_ref() == Some(&entry.relative);
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 12.0);
                ui.monospace(file_icon(&entry.relative));
                let mut label = egui::RichText::new(name);
                if is_preview {
                    label = label.italics();
                }
                let response = ui
                    .selectable_label(active == Some(normalized.as_str()), label)
                    .on_hover_text(&normalized);
                if let Some(menu) = context_menus::file_entry_menu(&response, &normalized, false) {
                    action = Some(match menu {
                        FileMenuAction::Open { path, .. } => FileTreeAction::Open(path),
                        menu => FileTreeAction::Menu(menu),
                    });
                } else if response.double_clicked() {
                    self.previewed = None;
                    action = Some(FileTreeAction::Open(normalized.clone()));
                } else if response.clicked() {
                    self.previewed = Some(entry.relative.clone());
                    action = Some(FileTreeAction::Preview(normalized.clone()));
                }
            });
        }

        action
    }
}

pub(crate) fn show(
    state: &mut FileTreeState,
    ui: &mut egui::Ui,
    active: Option<&str>,
) -> Option<FileTreeAction> {
    state.poll_scans();
    ui.horizontal(|ui| {
        ui.strong("Explorer");
        ui.label(format!("{} files", state.file_count()));
        if state.pending_scans > 0 {
            ui.spinner();
        }
    });
    ui.add(
        egui::TextEdit::singleline(&mut state.filter)
            .desired_width(f32::INFINITY)
            .hint_text("Filter loaded files"),
    );

    if state.pending_scans > 0 {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    let filter = state.filter.trim().to_ascii_lowercase();
    let max_height = (ui.available_height() - 34.0).max(120.0);
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("project_file_tree")
        .max_height(max_height)
        .show(ui, |ui| {
            action = state.show_directory(ui, Path::new(""), 0, active, &filter);
        });

    if let Some(error) = &state.last_error {
        ui.colored_label(
            crate::gui::theme::PALETTE.error,
            format!("Explorer: {error}"),
        );
    }
    action
}

fn spawn_scan(root: PathBuf, relative: PathBuf, sender: Sender<ScanResult>) {
    std::thread::spawn(move || {
        let result = scan_directory(&root, &relative);
        let _ = sender.send(ScanResult { relative, result });
    });
}

fn scan_directory(root: &Path, relative: &Path) -> io::Result<Vec<TreeEntry>> {
    let absolute = root.join(relative);
    let mut builder = WalkBuilder::new(&absolute);
    builder
        .max_depth(Some(1))
        .hidden(false)
        .follow_links(false)
        .parents(true)
        .require_git(false)
        .filter_entry(|entry| {
            entry.depth() == 0
                || !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | ".girder" | ".bitcode")
                )
        });

    let mut entries = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(|error| io::Error::other(error.to_string()))?;
        if entry.depth() != 1 {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| io::Error::other(error.to_string()))?
            .to_path_buf();
        entries.push(TreeEntry {
            relative,
            is_dir: entry.file_type().is_some_and(|kind| kind.is_dir()),
        });
    }
    entries.sort_by(|left, right| {
        right.is_dir.cmp(&left.is_dir).then_with(|| {
            normalized_relative(&left.relative)
                .to_ascii_lowercase()
                .cmp(&normalized_relative(&right.relative).to_ascii_lowercase())
        })
    });
    Ok(entries)
}

fn normalized_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn file_icon(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("rs") => "R",
        Some("py") => "Py",
        Some("js" | "jsx") => "JS",
        Some("ts" | "tsx") => "TS",
        Some("json" | "toml" | "yaml" | "yml") => "{}",
        Some("md" | "txt") => "¶",
        Some("html" | "htm") => "<>",
        Some("css") => "#",
        _ => "·",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_workspace(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "girder-file-tree-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create test workspace");
        root
    }

    #[test]
    fn one_level_scan_respects_gitignore_and_sorts_directories_first() {
        let root = temporary_workspace("ignore");
        fs::write(root.join(".gitignore"), "ignored.log\n").expect("write ignore file");
        fs::write(root.join("visible.rs"), "fn visible() {}\n").expect("write source");
        fs::write(root.join("ignored.log"), "ignored\n").expect("write ignored file");
        fs::create_dir(root.join("src")).expect("create directory");

        let entries = scan_directory(&root, Path::new("")).expect("scan directory");
        let names: Vec<_> = entries
            .iter()
            .map(|entry| normalized_relative(&entry.relative))
            .collect();
        assert_eq!(names.first().map(String::as_str), Some("src"));
        assert!(names.iter().any(|name| name == "visible.rs"));
        assert!(!names.iter().any(|name| name == "ignored.log"));

        fs::remove_dir_all(root).expect("remove test workspace");
    }

    #[test]
    fn file_icons_cover_common_editor_types() {
        assert_eq!(file_icon(Path::new("lib.rs")), "R");
        assert_eq!(file_icon(Path::new("main.py")), "Py");
        assert_eq!(file_icon(Path::new("app.tsx")), "TS");
        assert_eq!(file_icon(Path::new("README.md")), "¶");
    }
}
