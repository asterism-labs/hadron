//! Interactive TUI menuconfig for Kconfig-style configuration.
//!
//! Provides a terminal UI for browsing and editing build configuration
//! options, organized by menu categories. Saves overrides to `.hadron-config`.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

use crate::config;
use crate::model::{BuildModel, ConfigOptionDef, ConfigType, ConfigValue};

/// A menu entry in the TUI.
enum MenuEntry {
    /// Collapsible category header.
    Category { name: String, expanded: bool },
    /// A config option belonging to a category.
    Option { name: String },
}

/// State for an in-progress value edit.
struct EditState {
    option_name: String,
    buffer: String,
}

/// Main TUI application state.
struct App {
    model_options: BTreeMap<String, ConfigOptionDef>,
    values: BTreeMap<String, ConfigValue>,
    entries: Vec<MenuEntry>,
    cursor: usize,
    scroll_offset: usize,
    search_mode: bool,
    search_query: String,
    dirty: bool,
    editing: Option<EditState>,
    root: std::path::PathBuf,
}

impl App {
    fn new(model: &BuildModel, root: &Path, profile_name: &str) -> Result<Self> {
        let model_options = model.config_options.clone();

        // Load existing values: defaults → profile overrides → .hadron-config.
        let profile_overrides = collect_profile_config(model, profile_name)?;
        let file_overrides = config::load_config_overrides(root)?;
        let mut values = BTreeMap::new();
        for (name, opt) in &model_options {
            let val = file_overrides.get(name)
                .or_else(|| profile_overrides.get(name))
                .unwrap_or(&opt.default)
                .clone();
            values.insert(name.clone(), val);
        }

        // Build menu entries from menu_order.
        let mut entries = Vec::new();
        let mut categorized: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for (name, opt) in &model_options {
            let menu = opt.menu.as_deref().unwrap_or("General").to_string();
            categorized.entry(menu).or_default().push(name.clone());
        }

        // Use model.menu_order for ordering, then fall back to sorted keys.
        let mut ordered_menus: Vec<String> = model.menu_order.clone();
        for menu in categorized.keys() {
            if !ordered_menus.contains(menu) {
                ordered_menus.push(menu.clone());
            }
        }

        for menu in &ordered_menus {
            if let Some(options) = categorized.get(menu) {
                entries.push(MenuEntry::Category {
                    name: menu.clone(),
                    expanded: true,
                });
                for opt_name in options {
                    let opt_def = model_options.get(opt_name);
                    if opt_def.is_some_and(|o| o.ty == ConfigType::Group) {
                        // Group markers render as expandable sub-categories.
                        entries.push(MenuEntry::Category {
                            name: opt_name.clone(),
                            expanded: true,
                        });
                    } else {
                        entries.push(MenuEntry::Option {
                            name: opt_name.clone(),
                        });
                    }
                }
            }
        }

        Ok(App {
            model_options,
            values,
            entries,
            cursor: 0,
            scroll_offset: 0,
            search_mode: false,
            search_query: String::new(),
            dirty: false,
            editing: None,
            root: root.to_path_buf(),
        })
    }

    /// Get the visible entries (respecting collapsed categories).
    fn visible_entries(&self) -> Vec<(usize, &MenuEntry)> {
        let mut visible = Vec::new();
        let mut skip = false;

        for (i, entry) in self.entries.iter().enumerate() {
            match entry {
                MenuEntry::Category { expanded, .. } => {
                    visible.push((i, entry));
                    skip = !expanded;
                }
                MenuEntry::Option { name } => {
                    if skip {
                        continue;
                    }
                    // Filter by search query if active.
                    if !self.search_query.is_empty() {
                        let query = self.search_query.to_lowercase();
                        let matches = name.to_lowercase().contains(&query)
                            || self.model_options.get(name)
                                .and_then(|o| o.help.as_ref())
                                .map(|h| h.to_lowercase().contains(&query))
                                .unwrap_or(false);
                        if !matches {
                            continue;
                        }
                    }
                    visible.push((i, entry));
                }
            }
        }
        visible
    }

    /// Check if an option's dependencies are satisfied.
    fn deps_satisfied(&self, name: &str) -> bool {
        let Some(opt) = self.model_options.get(name) else {
            return true;
        };
        for dep in &opt.depends_on {
            match self.values.get(dep) {
                Some(ConfigValue::Bool(true)) => {}
                _ => return false,
            }
        }
        true
    }

    /// Toggle a boolean option, applying selects.
    fn toggle_bool(&mut self, name: &str) {
        if !self.deps_satisfied(name) {
            return;
        }
        if let Some(ConfigValue::Bool(v)) = self.values.get_mut(name) {
            *v = !*v;
            self.dirty = true;
            self.apply_selects();
        }
    }

    /// Cycle a Choice option forward or backward through its variants.
    fn cycle_choice(&mut self, name: &str, forward: bool) {
        if !self.deps_satisfied(name) {
            return;
        }
        let Some(opt) = self.model_options.get(name) else { return };
        let Some(ref choices) = opt.choices else { return };
        if choices.is_empty() {
            return;
        }

        let current = match self.values.get(name) {
            Some(ConfigValue::Choice(v)) => v.as_str(),
            Some(ConfigValue::Str(v)) => v.as_str(),
            _ => "",
        };

        let idx = choices.iter().position(|c| c == current).unwrap_or(0);
        let new_idx = if forward {
            (idx + 1) % choices.len()
        } else {
            (idx + choices.len() - 1) % choices.len()
        };

        self.values.insert(name.into(), ConfigValue::Choice(choices[new_idx].clone()));
        self.dirty = true;
    }

    /// Apply select and dependency propagation until stable.
    ///
    /// Pass 1: disable any enabled option whose `depends_on` is unsatisfied.
    /// Pass 2: enable any option forced on by another option's `selects`.
    /// Loop until no changes occur (fixed-point).
    fn apply_selects(&mut self) {
        loop {
            let mut changed = false;

            // Pass 1: propagate disables — if an option is enabled but a
            // dependency is off, force-disable it.
            for (name, opt) in &self.model_options {
                let is_enabled = matches!(self.values.get(name), Some(ConfigValue::Bool(true)));
                if is_enabled {
                    let deps_ok = opt.depends_on.iter().all(|dep| {
                        matches!(self.values.get(dep), Some(ConfigValue::Bool(true)))
                    });
                    if !deps_ok {
                        self.values.insert(name.clone(), ConfigValue::Bool(false));
                        changed = true;
                    }
                }
            }

            // Pass 2: propagate selects — if an option is enabled and selects
            // another, force-enable the selected option.
            for (name, opt) in &self.model_options {
                let is_enabled = matches!(self.values.get(name), Some(ConfigValue::Bool(true)));
                if is_enabled {
                    for selected in &opt.selects {
                        if let Some(ConfigValue::Bool(false)) = self.values.get(selected) {
                            self.values.insert(selected.clone(), ConfigValue::Bool(true));
                            changed = true;
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }
    }

    /// Get help text for the currently selected option.
    fn current_help(&self) -> (String, String) {
        let visible = self.visible_entries();
        if self.cursor >= visible.len() {
            return (String::new(), String::new());
        }
        let (_, entry) = &visible[self.cursor];
        match entry {
            MenuEntry::Category { name, .. } => {
                (name.clone(), format!("Category: {name}"))
            }
            MenuEntry::Option { name } => {
                let opt = self.model_options.get(name);
                let help = opt.and_then(|o| o.help.as_ref())
                    .cloned()
                    .unwrap_or_else(|| "No help available.".into());
                let type_str = opt.map(|o| format!("Type: {:?}", o.ty))
                    .unwrap_or_default();
                let default_str = opt.map(|o| format!("  Default: {:?}", o.default))
                    .unwrap_or_default();
                (help, format!("{type_str}{default_str}"))
            }
        }
    }

    /// Save current values to .hadron-config.
    fn save(&mut self) -> Result<()> {
        // Only save values that differ from defaults.
        let mut overrides = BTreeMap::new();
        for (name, val) in &self.values {
            if let Some(opt) = self.model_options.get(name) {
                if !config_value_eq(val, &opt.default) {
                    overrides.insert(name.clone(), val.clone());
                }
            }
        }
        config::save_config_overrides(&self.root, &overrides)?;
        self.dirty = false;
        Ok(())
    }

    /// Move cursor up.
    fn cursor_up(&mut self) {
        let visible = self.visible_entries();
        if self.cursor > 0 {
            self.cursor -= 1;
        } else if !visible.is_empty() {
            self.cursor = visible.len() - 1;
        }
    }

    /// Move cursor down.
    fn cursor_down(&mut self) {
        let visible = self.visible_entries();
        if self.cursor + 1 < visible.len() {
            self.cursor += 1;
        } else {
            self.cursor = 0;
        }
    }

    /// Toggle category expansion.
    fn toggle_category(&mut self) {
        let visible = self.visible_entries();
        if self.cursor >= visible.len() {
            return;
        }
        let (idx, _) = visible[self.cursor];
        if let MenuEntry::Category { ref mut expanded, .. } = self.entries[idx] {
            *expanded = !*expanded;
        }
    }

    /// Start editing the currently selected option.
    fn start_edit(&mut self) {
        // Extract what we need without holding a borrow on self.
        let action = {
            let visible = self.visible_entries();
            if self.cursor >= visible.len() {
                return;
            }
            let (_, entry) = &visible[self.cursor];
            if let MenuEntry::Option { name } = entry {
                let name = name.clone();
                if !self.deps_satisfied(&name) {
                    return;
                }
                let ty = self.model_options.get(&name).map(|o| o.ty);
                Some((name, ty))
            } else {
                None
            }
        };

        if let Some((name, ty)) = action {
            match ty {
                Some(ConfigType::Bool) => {
                    self.toggle_bool(&name);
                }
                Some(ConfigType::Choice) => {
                    // Enter/Space cycles forward for Choice options.
                    self.cycle_choice(&name, true);
                }
                Some(ConfigType::Group) => {
                    // Group markers are not directly editable.
                }
                Some(ConfigType::List) => {
                    // Open text editor pre-filled with comma-separated items.
                    let current = match self.values.get(&name) {
                        Some(ConfigValue::List(items)) => items.join(", "),
                        _ => String::new(),
                    };
                    self.editing = Some(EditState {
                        option_name: name,
                        buffer: current,
                    });
                }
                Some(_) => {
                    let current = self.values.get(&name).map(format_value).unwrap_or_default();
                    self.editing = Some(EditState {
                        option_name: name,
                        buffer: current,
                    });
                }
                None => {}
            }
        }
    }

    /// Commit the current edit.
    fn commit_edit(&mut self) {
        let Some(edit) = self.editing.take() else { return };
        let Some(opt) = self.model_options.get(&edit.option_name) else { return };

        let new_val = match opt.ty {
            ConfigType::U32 => edit.buffer.parse::<u32>().ok().map(ConfigValue::U32),
            ConfigType::U64 => {
                let s = edit.buffer.replace('_', "");
                if let Some(hex) = s.strip_prefix("0x") {
                    u64::from_str_radix(hex, 16).ok().map(ConfigValue::U64)
                } else {
                    s.parse::<u64>().ok().map(ConfigValue::U64)
                }
            }
            ConfigType::Str => Some(ConfigValue::Str(edit.buffer.clone())),
            ConfigType::Choice => Some(ConfigValue::Choice(edit.buffer.clone())),
            ConfigType::List => {
                let items: Vec<String> = edit
                    .buffer
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                Some(ConfigValue::List(items))
            }
            ConfigType::Bool | ConfigType::Group => unreachable!("bools/groups are not text-edited"),
        };

        if let Some(val) = new_val {
            self.values.insert(edit.option_name, val);
            self.dirty = true;
        }
    }
}

/// Run the interactive TUI menuconfig.
pub fn run_menuconfig(model: &BuildModel, root: &Path, profile_name: &str) -> Result<()> {
    let mut app = App::new(model, root, profile_name)?;

    // Set up terminal.
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, &mut app);

    // Restore terminal.
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw_ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            // Handle editing mode.
            if let Some(ref mut edit) = app.editing {
                match key.code {
                    KeyCode::Enter => {
                        app.commit_edit();
                    }
                    KeyCode::Esc => {
                        app.editing = None;
                    }
                    KeyCode::Backspace => {
                        edit.buffer.pop();
                    }
                    KeyCode::Char(c) => {
                        edit.buffer.push(c);
                    }
                    _ => {}
                }
                continue;
            }

            // Handle search mode.
            if app.search_mode {
                match key.code {
                    KeyCode::Enter | KeyCode::Esc => {
                        app.search_mode = false;
                    }
                    KeyCode::Backspace => {
                        app.search_query.pop();
                        app.cursor = 0;
                    }
                    KeyCode::Char(c) => {
                        app.search_query.push(c);
                        app.cursor = 0;
                    }
                    _ => {}
                }
                continue;
            }

            // Normal mode.
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    if app.dirty {
                        // Prompt to save. For simplicity, just save.
                        app.save()?;
                    }
                    return Ok(());
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Up | KeyCode::Char('k') => app.cursor_up(),
                KeyCode::Down | KeyCode::Char('j') => app.cursor_down(),
                KeyCode::Char(' ') => {
                    let action = {
                        let visible = app.visible_entries();
                        visible.get(app.cursor).map(|&(_, entry)| match entry {
                            MenuEntry::Category { .. } => None,
                            MenuEntry::Option { name } => Some(name.clone()),
                        })
                    };
                    match action {
                        Some(None) => app.toggle_category(),
                        Some(Some(ref name)) => {
                            let ty = app.model_options.get(name).map(|o| o.ty);
                            match ty {
                                Some(ConfigType::Choice) => app.cycle_choice(name, true),
                                _ => app.toggle_bool(name),
                            }
                        }
                        _ => {}
                    }
                }
                KeyCode::Left => {
                    // Left arrow cycles Choice backward.
                    let name = {
                        let visible = app.visible_entries();
                        visible.get(app.cursor).and_then(|&(_, entry)| match entry {
                            MenuEntry::Option { name } => Some(name.clone()),
                            _ => None,
                        })
                    };
                    if let Some(ref name) = name {
                        if app.model_options.get(name).is_some_and(|o| o.ty == ConfigType::Choice) {
                            app.cycle_choice(name, false);
                        }
                    }
                }
                KeyCode::Right => {
                    // Right arrow cycles Choice forward.
                    let name = {
                        let visible = app.visible_entries();
                        visible.get(app.cursor).and_then(|&(_, entry)| match entry {
                            MenuEntry::Option { name } => Some(name.clone()),
                            _ => None,
                        })
                    };
                    if let Some(ref name) = name {
                        if app.model_options.get(name).is_some_and(|o| o.ty == ConfigType::Choice) {
                            app.cycle_choice(name, true);
                        }
                    }
                }
                KeyCode::Enter => {
                    let is_category = {
                        let visible = app.visible_entries();
                        visible.get(app.cursor).map(|&(_, entry)| matches!(entry, MenuEntry::Category { .. }))
                    };
                    match is_category {
                        Some(true) => app.toggle_category(),
                        Some(false) => app.start_edit(),
                        _ => {}
                    }
                }
                KeyCode::Char('/') => {
                    app.search_mode = true;
                    app.search_query.clear();
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    app.save()?;
                }
                KeyCode::Esc => {
                    if !app.search_query.is_empty() {
                        app.search_query.clear();
                        app.cursor = 0;
                    }
                }
                _ => {}
            }
        }
    }
}

fn draw_ui(f: &mut ratatui::Frame, app: &mut App) {
    let size = f.area();

    // Layout: main list, help panel, key hints.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(5),
            Constraint::Length(3),
        ])
        .split(size);

    // Adjust scroll offset.
    let list_height = chunks[0].height.saturating_sub(2) as usize;
    if app.cursor < app.scroll_offset {
        app.scroll_offset = app.cursor;
    }
    if app.cursor >= app.scroll_offset + list_height {
        app.scroll_offset = app.cursor.saturating_sub(list_height - 1);
    }

    // Build list items.
    let visible = app.visible_entries();
    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .skip(app.scroll_offset)
        .take(list_height)
        .map(|(vi, (_, entry))| {
            let is_selected = vi == app.cursor;
            match entry {
                MenuEntry::Category { name, expanded } => {
                    let arrow = if *expanded { "▼" } else { "▶" };
                    let style = Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD);
                    let style = if is_selected {
                        style.bg(Color::DarkGray)
                    } else {
                        style
                    };
                    ListItem::new(Line::from(Span::styled(
                        format!("  {arrow} {name}"),
                        style,
                    )))
                }
                MenuEntry::Option { name } => {
                    let opt = app.model_options.get(name);
                    let val = app.values.get(name);
                    let deps_ok = app.deps_satisfied(name);

                    let val_str = match (opt.map(|o| o.ty), val) {
                        (Some(ConfigType::Bool), Some(ConfigValue::Bool(true))) => "[*]".to_string(),
                        (Some(ConfigType::Bool), _) => "[ ]".to_string(),
                        (Some(ConfigType::Choice), Some(ConfigValue::Choice(v))) => format!("< {v} >"),
                        (Some(ConfigType::List), Some(ConfigValue::List(items))) => {
                            format!("[{} items]", items.len())
                        }
                        (_, Some(v)) => format!("({})", format_value(v)),
                        _ => "(?)".to_string(),
                    };

                    let help_brief = opt
                        .and_then(|o| o.help.as_ref())
                        .map(|h| h.as_str())
                        .unwrap_or("");

                    let fg = if !deps_ok {
                        Color::DarkGray
                    } else if is_selected {
                        Color::White
                    } else {
                        Color::default()
                    };

                    let style = Style::default().fg(fg);
                    let style = if is_selected {
                        style.bg(Color::DarkGray)
                    } else {
                        style
                    };

                    // Extra indent for dotted sub-fields (group children).
                    let indent = if name.contains('.') { "          " } else { "      " };
                    let line = format!("{indent}{val_str} {name:<24} {help_brief}");
                    ListItem::new(Line::from(Span::styled(line, style)))
                }
            }
        })
        .collect();

    let title = if app.dirty {
        " Project Configuration [modified] "
    } else {
        " Project Configuration "
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, chunks[0]);

    // Help panel.
    let (help_text, help_meta) = app.current_help();
    let help_content = if let Some(ref edit) = app.editing {
        format!("Editing {}: {}_", edit.option_name, edit.buffer)
    } else if app.search_mode {
        format!("Search: {}_", app.search_query)
    } else {
        format!("{help_text}\n{help_meta}")
    };
    let help = Paragraph::new(help_content)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(Wrap { trim: true });
    f.render_widget(help, chunks[1]);

    // Key hints.
    let hints = if app.editing.is_some() {
        " Enter Confirm  Esc Cancel "
    } else if app.search_mode {
        " Type to search  Enter/Esc Done "
    } else {
        " ↑↓/jk Navigate  Space Toggle  ←→ Cycle Choice  Enter Edit  / Search  S Save  Q Quit "
    };
    let hints_widget = Paragraph::new(hints)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(hints_widget, chunks[2]);
}

/// Format a config value for display.
fn format_value(val: &ConfigValue) -> String {
    match val {
        ConfigValue::Bool(v) => format!("{v}"),
        ConfigValue::U32(v) => format!("{v}"),
        ConfigValue::U64(v) => format!("{v:#x}"),
        ConfigValue::Str(v) => v.clone(),
        ConfigValue::Choice(v) => v.clone(),
        ConfigValue::List(v) => v.join(", "),
    }
}

/// Compare two ConfigValues for equality.
fn config_value_eq(a: &ConfigValue, b: &ConfigValue) -> bool {
    match (a, b) {
        (ConfigValue::Bool(a), ConfigValue::Bool(b)) => a == b,
        (ConfigValue::U32(a), ConfigValue::U32(b)) => a == b,
        (ConfigValue::U64(a), ConfigValue::U64(b)) => a == b,
        (ConfigValue::Str(a), ConfigValue::Str(b)) => a == b,
        (ConfigValue::Choice(a), ConfigValue::Choice(b)) => a == b,
        (ConfigValue::List(a), ConfigValue::List(b)) => a == b,
        _ => false,
    }
}

/// Collect merged config overrides from a profile's inheritance chain.
fn collect_profile_config(
    model: &BuildModel,
    name: &str,
) -> Result<BTreeMap<String, ConfigValue>> {
    let Some(profile) = model.profiles.get(name) else {
        return Ok(BTreeMap::new());
    };

    let mut merged = if let Some(ref parent_name) = profile.inherits {
        collect_profile_config(model, parent_name)?
    } else {
        BTreeMap::new()
    };

    merged.extend(profile.config.clone());
    Ok(merged)
}
