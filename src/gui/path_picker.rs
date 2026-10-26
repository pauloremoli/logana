use crate::gui::{app::App, state::StatusMessage};
use crate::ingestion::archive_job::Control;
use gpui_kit::{
    assets::IconName,
    base::{Disableable, StyledExt, TestSupportExt},
    component::{
        ActiveTheme, Icon, Sizable, TitleBar,
        button::{Button, ButtonVariants},
        checkbox::Checkbox,
        h_resizable,
        input::{InputEvent, InputState},
        resizable_panel,
        switch::Switch,
    },
    gpui::{
        self, AnyWindowHandle, AppContext, Context, Entity, FocusHandle, IntoElement, Render, Task,
        WeakEntity, Window, div, prelude::*, px,
    },
};
use std::{collections::BTreeMap, path::PathBuf, sync::atomic::Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    Open,
    ArchiveOutput(u64),
}

struct Entry {
    path: PathBuf,
    name: String,
    directory: bool,
}
struct Listing {
    directory: PathBuf,
    entries: Vec<Entry>,
    file: Option<PathBuf>,
    warning: Option<String>,
}

pub(super) struct PathPicker {
    pub window: AnyWindowHandle,
    owner: WeakEntity<App>,
    purpose: Purpose,
    directory: PathBuf,
    entries: Vec<Entry>,
    rows: Vec<usize>,
    selected: BTreeMap<PathBuf, bool>,
    path_input: Entity<InputState>,
    search: Entity<InputState>,
    hidden: bool,
    merge: bool,
    cursor: usize,
    focus: FocusHandle,
    pub(super) loading: bool,
    ready: bool,
    scroll: gpui::ScrollHandle,
    error: Option<String>,
    scan: Control,
    task: Option<Task<()>>,
}

fn display_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_owned()
    } else {
        value.into_owned()
    }
}

fn read_directory(
    path: PathBuf,
    directories_only: bool,
    control: &Control,
) -> Result<Listing, String> {
    control.check().map_err(|e| e.to_string())?;
    let path =
        std::fs::canonicalize(&path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    let file = if metadata.is_file() && !directories_only {
        Some(path.clone())
    } else {
        None
    };
    if !metadata.is_dir() && file.is_none() {
        return Err("Choose a folder.".into());
    }
    let directory = if file.is_some() {
        path.parent().unwrap().to_path_buf()
    } else {
        path
    };
    let mut entries = Vec::new();
    let mut warning = None;
    for entry in std::fs::read_dir(&directory)
        .map_err(|e| format!("Cannot read {}: {e}", directory.display()))?
    {
        control.check().map_err(|e| e.to_string())?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warning = Some(error.to_string());
                continue;
            }
        };
        let metadata = match std::fs::metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) => {
                warning = Some(format!(
                    "Cannot inspect {}: {error}",
                    entry.path().display()
                ));
                continue;
            }
        };
        if metadata.is_dir() || (metadata.is_file() && !directories_only) {
            entries.push(Entry {
                path: entry.path(),
                name: entry.file_name().to_string_lossy().into_owned(),
                directory: metadata.is_dir(),
            });
        }
    }
    entries.sort_by_key(|e| (!e.directory, e.name.to_lowercase(), e.name.clone()));
    Ok(Listing {
        directory,
        entries,
        file,
        warning,
    })
}

fn rotation_preview(selected: &BTreeMap<PathBuf, bool>, merge: bool) -> Vec<(PathBuf, bool, bool)> {
    let mut groups = BTreeMap::<PathBuf, Vec<(u64, PathBuf)>>::new();
    let mut rows = Vec::new();
    for (path, directory) in selected {
        let kind = crate::ingestion::detect_archive_type(&path.to_string_lossy());
        if !merge
            || *directory
            || kind.is_some_and(|kind| {
                !matches!(
                    kind,
                    crate::ingestion::ArchiveType::Gz
                        | crate::ingestion::ArchiveType::Bz2
                        | crate::ingestion::ArchiveType::Xz
                )
            })
        {
            rows.push((path.clone(), false, false));
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if let Some((base, rank)) = crate::shared::archive_selection::rotation_name(&name) {
            groups
                .entry(path.with_file_name(base))
                .or_default()
                .push((rank, path.clone()));
        } else {
            rows.push((path.clone(), false, false));
        }
    }
    for (base, mut members) in groups {
        if members.len() < 2 || !members.iter().any(|(rank, _)| *rank > 0) {
            rows.extend(members.into_iter().map(|(_, path)| (path, false, false)));
            continue;
        }
        let mut name = base.file_stem().unwrap_or_default().to_os_string();
        name.push("-merged");
        if let Some(ext) = base.extension() {
            name.push(".");
            name.push(ext);
        }
        rows.push((base.with_file_name(name), false, true));
        members.sort_by_key(|(rank, path)| (std::cmp::Reverse(*rank), path.clone()));
        rows.extend(members.into_iter().map(|(_, path)| (path, true, false)));
    }
    rows
}

impl Drop for PathPicker {
    fn drop(&mut self) {
        self.scan.cancel.store(true, Ordering::Relaxed);
    }
}

impl PathPicker {
    fn new(
        owner: WeakEntity<App>,
        purpose: Purpose,
        directory: PathBuf,
        merge: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Folder or file path"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter this folder…"));
        cx.subscribe(&path_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.navigate_text(input.read(cx).value().to_string(), cx);
            }
        })
        .detach();
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
                cx.notify();
            }
        })
        .detach();
        let mut this = Self {
            window: window.window_handle(),
            owner,
            purpose,
            directory: directory.clone(),
            entries: Vec::new(),
            rows: Vec::new(),
            selected: BTreeMap::new(),
            path_input,
            search,
            hidden: false,
            merge,
            cursor: 0,
            focus: cx.focus_handle(),
            loading: false,
            ready: false,
            scroll: gpui::ScrollHandle::new(),
            error: None,
            scan: Control::new().0,
            task: None,
        };
        this.focus.focus(window, cx);
        this.navigate(directory, cx);
        this
    }

    #[cfg(all(test, feature = "gui-test"))]
    pub(super) fn choose_test_directory(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.navigate(directory, cx);
    }

    fn navigate_text(&mut self, value: String, cx: &mut Context<Self>) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        let path = if value == "~" {
            dirs::home_dir().unwrap_or_else(|| self.directory.clone())
        } else if let Some(rest) = value.strip_prefix("~/") {
            dirs::home_dir().unwrap_or_default().join(rest)
        } else {
            PathBuf::from(value)
        };
        self.navigate(
            if path.is_absolute() {
                path
            } else {
                self.directory.join(path)
            },
            cx,
        );
    }

    fn navigate(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.scan.cancel.store(true, Ordering::Relaxed);
        self.scan = Control::new().0;
        let control = self.scan.clone();
        let directories_only = self.purpose != Purpose::Open;
        self.loading = true;
        self.error = None;
        let work = cx
            .background_executor()
            .spawn(async move { read_directory(directory, directories_only, &control) });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(listing) => {
                        this.directory = listing.directory;
                        if this.purpose == Purpose::Open {
                            let directory = this.directory.clone();
                            this.owner
                                .update(cx, |app, _| {
                                    let mut prefs = app.state.preferences.clone();
                                    prefs.last_open_directory = Some(directory);
                                    app.save_preferences(prefs);
                                })
                                .ok();
                        }
                        this.entries = listing.entries;
                        this.ready = true;
                        this.error = listing.warning;
                        if let Some(file) = listing.file {
                            this.selected.insert(file, false);
                        }
                        this.path_input.update(cx, |input, cx| {
                            input.set_value(display_path(&this.directory), window, cx)
                        });
                        this.refresh(cx);
                        this.focus.focus(window, cx);
                        this.scroll.scroll_to_item(0);
                    }
                    Err(error) => {
                        this.error = Some(error);
                        this.path_input.update(cx, |input, cx| {
                            input.set_value(display_path(&this.directory), window, cx)
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn refresh(&mut self, cx: &Context<Self>) {
        let query = self.search.read(cx).value().to_lowercase();
        self.rows = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (self.hidden || !e.name.starts_with('.')) && e.name.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    fn toggle(&mut self, path: PathBuf, directory: bool, cx: &mut Context<Self>) {
        if self.selected.remove(&path).is_none() {
            if self.purpose != Purpose::Open {
                self.selected.clear();
            }
            self.selected.insert(path, directory);
        }
        self.refresh(cx);
        cx.notify();
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.selected
            .keys()
            .filter(|path| {
                !path
                    .ancestors()
                    .skip(1)
                    .any(|parent| self.selected.get(parent) == Some(&true))
            })
            .cloned()
            .collect()
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || !self.ready {
            return;
        }
        let paths = self.roots();
        match self.purpose {
            Purpose::Open if paths.is_empty() => return,
            Purpose::Open => {
                let merge = self.merge;
                self.owner
                    .update(cx, |app, cx| {
                        let mut prefs = app.state.preferences.clone();
                        prefs.archive_merge_rotations = merge;
                        app.save_preferences(prefs);
                        if merge {
                            app.queue_rotation_selection(paths, cx);
                        } else {
                            for path in paths {
                                app.queue_file(path, cx);
                            }
                        }
                    })
                    .ok();
            }
            Purpose::ArchiveOutput(id) => {
                let path = paths
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| self.directory.clone());
                self.owner
                    .update(cx, |app, cx| {
                        if let Some(picker) = app
                            .loading
                            .operations
                            .iter_mut()
                            .find(|op| op.id == id)
                            .and_then(|op| op.picker.as_mut())
                        {
                            picker.output_dir = path;
                        }
                        cx.notify();
                    })
                    .ok();
            }
        }
        window.remove_window();
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            window.remove_window();
            cx.stop_propagation();
            return;
        }
        if event.keystroke.key == "enter"
            && (event.keystroke.modifiers.control || event.keystroke.modifiers.platform)
        {
            self.confirm(window, cx);
            cx.stop_propagation();
            return;
        }
        if !self.focus.is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => self.cursor = self.cursor.saturating_sub(1),
            "down" => self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1)),
            "space" => {
                if let Some(&index) = self.rows.get(self.cursor) {
                    let e = &self.entries[index];
                    self.toggle(e.path.clone(), e.directory, cx);
                }
            }
            "enter" | "right" => {
                if let Some(&index) = self.rows.get(self.cursor) {
                    let e = &self.entries[index];
                    if e.directory {
                        self.navigate(e.path.clone(), cx);
                    } else {
                        self.toggle(e.path.clone(), false, cx);
                    }
                }
            }
            "left" | "backspace" => {
                if let Some(parent) = self.directory.parent() {
                    self.navigate(parent.to_path_buf(), cx);
                }
            }
            _ => return,
        }
        self.scroll.scroll_to_item(self.cursor);
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for PathPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let directory_mode = self.purpose != Purpose::Open;
        let list = div()
            .id("path-entries")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .children(self.rows.iter().copied().enumerate().map(|(row, index)| {
                let entry = &self.entries[index];
                let path = entry.path.clone();
                let browse = entry.path.clone();
                let is_dir = entry.directory;
                div()
                    .id(("path-row", index))
                    .test_support()
                    .w_full()
                    .min_w(px(0.))
                    .min_h(px(36.))
                    .flex_shrink_0()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .when(self.cursor == row, |d| d.bg(cx.theme().secondary))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.cursor = row;
                            this.focus.focus(window, cx);
                            cx.notify();
                        }),
                    )
                    .child(
                        Checkbox::new(("path-check", index))
                            .checked(self.selected.contains_key(&path))
                            .accessibility_label(format!("Select {}", entry.name))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle(path.clone(), is_dir, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(("path-icon", index))
                            .test_support()
                            .flex_shrink_0()
                            .child(
                                Icon::new(if is_dir {
                                    IconName::Folder
                                } else {
                                    IconName::File
                                })
                                .size_4()
                                .text_color(if is_dir {
                                    cx.theme().warning
                                } else {
                                    cx.theme().primary
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id(("path-name", index))
                            .test_support()
                            .flex_1()
                            .min_w(px(0.))
                            .whitespace_normal()
                            .cursor_pointer()
                            .child(entry.name.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if is_dir {
                                    this.navigate(browse.clone(), cx);
                                } else {
                                    this.toggle(browse.clone(), false, cx);
                                }
                            })),
                    )
            }));
        let selected = div()
            .id("path-selected-list")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(
                rotation_preview(&self.selected, self.merge)
                    .into_iter()
                    .map(|(path, child, output)| {
                        let remove = path.clone();
                        div()
                            .id(format!(
                                "{}:{}",
                                if output {
                                    "path-output"
                                } else {
                                    "path-selected"
                                },
                                path.display()
                            ))
                            .test_support()
                            .w_full()
                            .min_w(px(0.))
                            .min_h(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .py_2()
                            .when(child, |d| d.pl_4())
                            .when(output, |d| d.font_semibold())
                            .child(
                                div()
                                    .id(format!(
                                        "{}:{}",
                                        if output {
                                            "path-output-label"
                                        } else {
                                            "path-label"
                                        },
                                        path.display()
                                    ))
                                    .test_support()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_normal()
                                    .child(display_path(&path)),
                            )
                            .when(!output, |d| {
                                d.child(
                                    Button::new(format!("path-remove:{}", path.display()))
                                        .ghost()
                                        .flex_shrink_0()
                                        .small()
                                        .icon(IconName::X)
                                        .accessibility_label(format!("Remove {}", path.display()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.selected.remove(&remove);
                                            this.refresh(cx);
                                            cx.notify();
                                        })),
                                )
                            })
                    }),
            );
        let files = div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .pr_3()
            .min_w(px(0.))
            .child(
                crate::gui::view::input(&self.search)
                    .id("path-search")
                    .aria_label("Filter filenames")
                    .cleanable(true),
            )
            .child(
                div().flex().items_center().gap_2().child(
                    Switch::new("path-hidden")
                        .small()
                        .label("Show hidden files")
                        .checked(self.hidden)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.hidden = !this.hidden;
                            this.refresh(cx);
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .when(self.rows.is_empty() && !self.loading, |d| {
                        d.child("No matching files or folders.")
                    })
                    .child(list),
            );
        let selection = div()
            .id("path-selection-pane")
            .test_support()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .pl_3()
            .min_w(px(0.))
            .child(
                div()
                    .font_semibold()
                    .child(format!("Selected paths ({})", self.selected.len())),
            )
            .child(div().flex_1().min_h(px(0.)).child(selected))
            .child(
                Button::new("path-clear")
                    .small()
                    .label("Clear selection")
                    .disabled(self.selected.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected.clear();
                        this.refresh(cx);
                        cx.notify();
                    })),
            );
        let count = self.roots().len();
        div()
            .id("path-picker")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .child(
                TitleBar::new()
                    .child(if directory_mode {
                        "Choose output folder"
                    } else {
                        "Open files and folders"
                    })
                    .on_close_window(cx.listener(|_, _, window, _| window.remove_window())),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Button::new("path-home").small().label("Home").on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(home) = dirs::home_dir() {
                                        this.navigate(home, cx);
                                    }
                                }),
                            ))
                            .child(
                                Button::new("path-parent")
                                    .small()
                                    .label("Up")
                                    .disabled(self.directory.parent().is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(parent) = this.directory.parent() {
                                            this.navigate(parent.to_path_buf(), cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("path-refresh")
                                    .small()
                                    .label("Refresh")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.navigate(this.directory.clone(), cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                crate::gui::view::input(&self.path_input)
                                    .id("path-location")
                                    .aria_label("Folder or file path")
                                    .flex_1(),
                            )
                            .child(Button::new("path-go").small().label("Go").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.navigate_text(
                                        this.path_input.read(cx).value().to_string(),
                                        cx,
                                    )
                                }),
                            )),
                    )
                    .when(self.loading, |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Reading folder…"),
                        )
                    })
                    .when_some(self.error.clone(), |d, error| {
                        d.child(
                            div()
                                .id("path-error")
                                .test_support()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    })
                    .child(
                        div().flex_1().min_h(px(0.)).child(
                            h_resizable("path-picker-panes")
                                .child(
                                    resizable_panel()
                                        .size(px(500.))
                                        .size_range(px(240.)..px(2000.))
                                        .child(files),
                                )
                                .child(
                                    resizable_panel()
                                        .size(px(320.))
                                        .size_range(px(200.)..px(2000.))
                                        .child(selection),
                                ),
                        ),
                    )
                    .when(!directory_mode, |d| {
                        d.child(
                            Button::new("path-select-folder")
                                .small()
                                .label("Select current folder")
                                .disabled(self.loading)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle(this.directory.clone(), true, cx)
                                })),
                        )
                    })
                    .when(!directory_mode, |d| {
                        d.child(
                            Switch::new("path-merge")
                                .label("Auto merge rotated logs")
                                .checked(self.merge)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.merge = !this.merge;
                                    this.owner
                                        .update(cx, |app, _| {
                                            let mut prefs = app.state.preferences.clone();
                                            prefs.archive_merge_rotations = this.merge;
                                            app.save_preferences(prefs);
                                        })
                                        .ok();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("path-confirm")
                                    .primary()
                                    .label(if directory_mode && count > 0 {
                                        "Use selected folder".into()
                                    } else if directory_mode {
                                        "Use this folder".into()
                                    } else {
                                        format!("Open {count} selected paths")
                                    })
                                    .disabled(
                                        self.loading
                                            || !self.ready
                                            || (!directory_mode && count == 0),
                                    )
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("path-cancel").label("Cancel").on_click(
                                    cx.listener(|_, _, window, _| window.remove_window()),
                                ),
                            ),
                    ),
            )
            .on_key_down(cx.listener(Self::key))
            .into_any_element()
    }
}

impl App {
    pub(super) fn show_path_picker(
        &mut self,
        purpose: Purpose,
        initial: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = self.path_picker.as_ref().and_then(WeakEntity::upgrade)
            && picker.read(cx).purpose == purpose
            && picker
                .read(cx)
                .window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let initial = if purpose == Purpose::Open {
            self.state
                .preferences
                .last_open_directory
                .clone()
                .filter(|path| path.is_dir())
                .unwrap_or(initial)
        } else {
            initial
        };
        let merge = self.state.preferences.archive_merge_rotations;
        let owner = cx.entity().downgrade();
        let options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                None,
                gpui::size(px(960.), px(720.)),
                cx,
            ))),
            window_min_size: Some(gpui::size(px(620.), px(560.))),
            ..TitleBar::window_options()
        };
        match gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| PathPicker::new(owner, purpose, initial, merge, window, cx))
        }) {
            Ok((_, picker)) => self.path_picker = Some(picker.downgrade()),
            Err(error) => self.state.notify(StatusMessage::Error(format!(
                "Cannot open file picker: {error}"
            ))),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_windows_paths_without_extended_prefixes() {
        for (path, expected) in [
            (r"\\?\C:\Program Files", r"C:\Program Files"),
            (r"\\?\UNC\server\share\logs", r"\\server\share\logs"),
            (r"C:\logs", r"C:\logs"),
            ("/tmp/logs", "/tmp/logs"),
            (r"\\?\Volume{abc}\", r"\\?\Volume{abc}\"),
        ] {
            assert_eq!(display_path(std::path::Path::new(path)), expected);
        }
    }

    #[test]
    fn preview_groups_rotations_beneath_the_output() {
        let selected = [
            ("/logs/runtime.log", false),
            ("/logs/runtime.log1", false),
            ("/logs/other.log", false),
        ]
        .into_iter()
        .map(|(p, d)| (PathBuf::from(p), d))
        .collect();
        let rows = rotation_preview(&selected, true);
        let parent = rows
            .iter()
            .position(|(path, _, output)| *output && path.ends_with("runtime-merged.log"))
            .unwrap();
        assert_eq!(
            rows[parent + 1],
            (PathBuf::from("/logs/runtime.log1"), true, false)
        );
        assert_eq!(
            rows[parent + 2],
            (PathBuf::from("/logs/runtime.log"), true, false)
        );
        assert!(rows.contains(&(PathBuf::from("/logs/other.log"), false, false)));
        assert!(
            rotation_preview(&selected, false)
                .iter()
                .all(|(_, child, output)| !child && !output)
        );
        let remaining = selected
            .into_iter()
            .filter(|(p, _)| !p.ends_with("runtime.log1"))
            .collect();
        assert!(
            rotation_preview(&remaining, true)
                .iter()
                .all(|(_, child, output)| !child && !output)
        );
    }

    #[test]
    fn preview_keeps_an_existing_merged_source_separate_from_the_output() {
        let selected = [
            "/logs/runtime.log",
            "/logs/runtime.log1",
            "/logs/runtime-merged.log",
        ]
        .into_iter()
        .map(|p| (PathBuf::from(p), false))
        .collect();
        let rows = rotation_preview(&selected, true);
        let same_name: Vec<_> = rows
            .iter()
            .filter(|(path, _, _)| path.ends_with("runtime-merged.log"))
            .collect();
        assert_eq!(same_name.len(), 2);
        assert_eq!(same_name.iter().filter(|(_, _, output)| *output).count(), 1);
        assert_eq!(
            same_name.iter().filter(|(_, _, output)| !*output).count(),
            1
        );
    }

    #[test]
    fn listing_sorts_folders_and_accepts_a_typed_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("z-folder")).unwrap();
        let file = dir.path().join("a.log");
        std::fs::write(&file, "dummy").unwrap();
        let (control, _) = Control::new();
        let listing = read_directory(file.clone(), false, &control).unwrap();
        assert_eq!(listing.file, Some(file));
        assert!(listing.entries[0].directory);
        let folders = read_directory(dir.path().into(), true, &control).unwrap();
        assert_eq!(folders.entries.len(), 1);
        assert!(read_directory(dir.path().join("a.log"), true, &control).is_err());
        control.cancel.store(true, Ordering::Relaxed);
        assert!(read_directory(dir.path().into(), false, &control).is_err());
    }
}

#[cfg(all(test, feature = "gui-test"))]
mod ui_tests {
    use super::*;
    use gpui_kit::gpui::TestAppContext;
    use std::sync::Arc;

    #[gpui_kit::test]
    async fn selected_folder_and_child_merge_only_rotations(cx: &mut TestAppContext) {
        use gpui_kit::test::{TestAppContextExt, TestWindowExt};
        use std::time::Duration;
        cx.background_executor.allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let first = dir
            .path()
            .join(format!("{}-first.log", "long-folder-name-".repeat(8)));
        std::fs::write(&first, "Jan  1 00:00:02 host app: second\n").unwrap();
        std::fs::write(
            format!("{}1", first.display()),
            "Jan  1 00:00:01 host app: previous\n",
        )
        .unwrap();
        std::fs::write(
            nested.join("second.log"),
            "Jan  1 00:00:01 host app: first\n",
        )
        .unwrap();
        let db = Arc::new(
            crate::gui::runtime::handle()
                .block_on(crate::db::Database::in_memory())
                .unwrap(),
        );
        let theme = crate::theme::Theme::default();
        let (main, app) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::gui::theme::install(&theme, cx);
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| App::new(db, theme, cx))
            })
            .unwrap()
        });
        app.update(cx, |app, cx| {
            app.state.preferences.archive_merge_rotations = false;
            app.show_path_picker(Purpose::Open, dir.path().into(), cx)
        });
        let picker = app.read_with(cx, |app, _| {
            app.path_picker.as_ref().unwrap().upgrade().unwrap()
        });
        let handle = picker.read_with(cx, |picker, _| picker.window);
        cx.wait_for(handle, Duration::from_secs(30), |_, cx| {
            !picker.read(cx).loading
        })
        .await;
        cx.update_window(handle, |_, window, cx| {
            picker.update(cx, |picker, cx| {
                picker.toggle(dir.path().into(), true, cx);
                picker.toggle(first.clone(), false, cx);
                assert_eq!(picker.roots(), vec![dir.path().to_path_buf()]);
            });
            window.render_frame(cx);
            let entry_index = picker
                .read(cx)
                .entries
                .iter()
                .position(|e| e.path == first)
                .unwrap();
            assert_eq!(
                app.read(cx)
                    .state
                    .preferences
                    .last_open_directory
                    .as_deref(),
                Some(dir.path())
            );
            let files = window.find("path-entries").bounds();
            let search = window.find("path-search").bounds();
            assert!(files.size.width >= search.size.width - px(1.));
            let entry_label = window.find(("path-name", entry_index)).bounds();
            let icon = window.find(("path-icon", entry_index)).bounds();
            assert!(icon.right() <= entry_label.left());
            let entry_row = window.find(("path-row", entry_index)).bounds();
            assert!(entry_label.size.height > px(36.), "left filenames wrap");
            assert!(entry_row.size.height >= entry_label.size.height);
            let row = window
                .find(format!("path-selected:{}", first.display()))
                .bounds();
            let remove = window
                .find(format!("path-remove:{}", first.display()))
                .bounds();
            let picker_bounds = window.find("path-picker").bounds();
            let pane = window.find("path-selection-pane").bounds();
            let label = window
                .find(format!("path-label:{}", first.display()))
                .bounds();
            assert!(row.size.width >= pane.size.width - px(16.));
            assert!(label.size.width >= row.size.width - px(60.));
            assert!(
                label.size.height > px(36.),
                "long paths wrap instead of truncating"
            );
            assert!(row.size.height >= label.size.height);
            assert!(row.right() <= picker_bounds.right());
            assert!(remove.right() <= row.right());
            assert!(remove.left() >= row.left());
            assert!(!picker.read(cx).merge);
            window.click("path-merge", cx);
            assert!(picker.read(cx).merge);
            assert!(app.read(cx).state.preferences.archive_merge_rotations);
            window.render_frame(cx);
            window.click("path-confirm", cx);
        })
        .unwrap();
        assert!(!cx.did_prompt_for_paths());
        cx.wait_for(main, Duration::from_secs(30), |_, cx| {
            app.read(cx).loading.operations.is_empty()
        })
        .await;
        app.read_with(cx, |app, _| {
            assert_eq!(app.state.tabs.len(), 2, "{:?}", app.state.status);
            assert!(app.state.tabs.iter().any(|tab| {
                tab.file_reader.line_count() == 2
                    && tab
                        .log_manager
                        .source_file()
                        .unwrap()
                        .contains("-merged.log")
            }));
            assert!(
                app.state
                    .tabs
                    .iter()
                    .any(|tab| tab.file_reader.line_count() == 1)
            );
        });
    }
}
