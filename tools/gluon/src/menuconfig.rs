//! Interactive TUI menuconfig for Kconfig-style configuration.
//!
//! Provides a hierarchical, page-based terminal UI (Linux kernel style) for
//! browsing and editing build configuration options. Saves overrides to
//! `.hadron-config`.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

use crate::config;
use crate::model::{BuildModel, ConfigOptionDef, ConfigType, ConfigValue};

// ─── Hierarchical menu tree ──────────────────────────────────────────

/// A node in the hierarchical menu tree (submenu page).
struct MenuNode {
    title: String,
    children: Vec<MenuChild>,
}

/// An entry within a menu page — either a submenu link or a config option.
enum MenuChild {
    /// Navigate into a sub-page.
    SubMenu(MenuNode),
    /// A leaf config option.
    Option { name: String },
}

/// Tracks the current page position within a `MenuNode`.
struct Page {
    /// Path of child indices from root to this page's parent.
    path: Vec<usize>,
    cursor: usize,
    scroll_offset: usize,
}

/// Stack of pages for hierarchical navigation (back = pop).
struct NavStack {
    pages: Vec<Page>,
}

impl NavStack {
    fn new() -> Self {
        Self {
            pages: vec![Page {
                path: vec![],
                cursor: 0,
                scroll_offset: 0,
            }],
        }
    }

    fn current(&self) -> &Page {
        self.pages.last().expect("nav stack is never empty")
    }

    fn current_mut(&mut self) -> &mut Page {
        self.pages.last_mut().expect("nav stack is never empty")
    }

    /// Push a new page when entering a submenu.
    fn push(&mut self, child_index: usize) {
        let mut path = self.current().path.clone();
        path.push(child_index);
        self.pages.push(Page {
            path,
            cursor: 0,
            scroll_offset: 0,
        });
    }

    /// Pop back to the parent page. Returns false if already at root.
    fn pop(&mut self) -> bool {
        if self.pages.len() > 1 {
            self.pages.pop();
            true
        } else {
            false
        }
    }

    /// Build a breadcrumb trail from the current navigation path.
    fn breadcrumbs(&self, root: &MenuNode) -> Vec<String> {
        let mut crumbs = vec![root.title.clone()];
        let current = self.current();
        let mut node = root;
        for &idx in &current.path {
            if let Some(MenuChild::SubMenu(sub)) = node.children.get(idx) {
                crumbs.push(sub.title.clone());
                node = sub;
            }
        }
        crumbs
    }
}

/// Reverse dependency information for help popups.
struct ReverseDeps {
    /// Options that depend on a given symbol (symbol → dependents).
    depended_on_by: BTreeMap<String, Vec<String>>,
    /// Options that are selected by a given symbol (symbol → selectors).
    selected_by: BTreeMap<String, Vec<String>>,
}

/// State for the help popup overlay.
struct HelpPopupState {
    option_name: String,
    scroll_offset: usize,
}

/// State for an in-progress value edit.
struct EditState {
    option_name: String,
    buffer: String,
}

/// A search result pointing to an option anywhere in the tree.
struct SearchResult {
    /// Path of child indices to the menu containing this option.
    path: Vec<usize>,
    /// Index within the containing menu's children.
    child_index: usize,
    /// Config option name.
    name: String,
}

// ─── Tree builder ────────────────────────────────────────────────────

/// Build a `MenuNode` tree from the fully-expanded Kconfig AST.
fn build_menu_tree(ast: &crate::kconfig::ast::KconfigFile) -> MenuNode {
    fn convert_items(items: &[crate::kconfig::ast::KconfigItem]) -> Vec<MenuChild> {
        let mut children = Vec::new();
        for item in items {
            match item {
                crate::kconfig::ast::KconfigItem::Config(block) => {
                    children.push(MenuChild::Option {
                        name: block.name.clone(),
                    });
                }
                crate::kconfig::ast::KconfigItem::Menu(menu) => {
                    children.push(MenuChild::SubMenu(MenuNode {
                        title: menu.title.clone(),
                        children: convert_items(&menu.items),
                    }));
                }
                // Source directives are already resolved; Presets are not menu items.
                _ => {}
            }
        }
        children
    }

    MenuNode {
        title: "Main Menu".to_string(),
        children: convert_items(&ast.items),
    }
}

/// Compute reverse dependency maps from the config option definitions.
fn build_reverse_deps(options: &BTreeMap<String, ConfigOptionDef>) -> ReverseDeps {
    let mut depended_on_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut selected_by: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (name, opt) in options {
        for dep in &opt.depends_on {
            depended_on_by
                .entry(dep.clone())
                .or_default()
                .push(name.clone());
        }
        for sel in &opt.selects {
            selected_by
                .entry(sel.clone())
                .or_default()
                .push(name.clone());
        }
    }

    ReverseDeps {
        depended_on_by,
        selected_by,
    }
}

/// Resolve the current `MenuNode` from the root given a navigation path.
fn resolve_current_node<'a>(root: &'a MenuNode, path: &[usize]) -> &'a MenuNode {
    let mut node = root;
    for &idx in path {
        if let Some(MenuChild::SubMenu(sub)) = node.children.get(idx) {
            node = sub;
        }
    }
    node
}

/// Recursively search the tree for options matching a query.
fn search_tree(
    node: &MenuNode,
    query: &str,
    options: &BTreeMap<String, ConfigOptionDef>,
    path: &[usize],
    results: &mut Vec<SearchResult>,
) {
    let query_lower = query.to_lowercase();
    for (i, child) in node.children.iter().enumerate() {
        match child {
            MenuChild::Option { name } => {
                let matches = name.to_lowercase().contains(&query_lower)
                    || options
                        .get(name)
                        .and_then(|o| o.help.as_ref())
                        .is_some_and(|h| h.to_lowercase().contains(&query_lower));
                if matches {
                    results.push(SearchResult {
                        path: path.to_vec(),
                        child_index: i,
                        name: name.clone(),
                    });
                }
            }
            MenuChild::SubMenu(sub) => {
                let mut sub_path = path.to_vec();
                sub_path.push(i);
                search_tree(sub, query, options, &sub_path, results);
            }
        }
    }
}

// ─── Application state ──────────────────────────────────────────────

/// Main TUI application state.
struct App {
    model_options: BTreeMap<String, ConfigOptionDef>,
    values: BTreeMap<String, ConfigValue>,
    menu_tree: MenuNode,
    nav: NavStack,
    reverse_deps: ReverseDeps,
    help_popup: Option<HelpPopupState>,
    search_mode: bool,
    search_query: String,
    search_results: Vec<SearchResult>,
    search_cursor: usize,
    dirty: bool,
    editing: Option<EditState>,
    root: std::path::PathBuf,
}

impl App {
    fn new(
        model: &BuildModel,
        root: &Path,
        profile_name: &str,
        kconfig_tree: &crate::kconfig::ast::KconfigFile,
    ) -> Result<Self> {
        let model_options = model.config_options.clone();

        // Load existing values: defaults → profile overrides → .hadron-config.
        let profile_overrides = collect_profile_config(model, profile_name)?;
        let file_overrides = config::load_config_overrides(root)?;
        let mut values = BTreeMap::new();
        for (name, opt) in &model_options {
            let val = file_overrides
                .get(name)
                .or_else(|| profile_overrides.get(name))
                .unwrap_or(&opt.default)
                .clone();
            values.insert(name.clone(), val);
        }

        let menu_tree = build_menu_tree(kconfig_tree);
        let reverse_deps = build_reverse_deps(&model_options);

        Ok(App {
            model_options,
            values,
            menu_tree,
            nav: NavStack::new(),
            reverse_deps,
            help_popup: None,
            search_mode: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_cursor: 0,
            dirty: false,
            editing: None,
            root: root.to_path_buf(),
        })
    }

    /// Get the children of the currently displayed page.
    fn current_children(&self) -> &[MenuChild] {
        let path = &self.nav.current().path;
        let node = resolve_current_node(&self.menu_tree, path);
        &node.children
    }

    /// Get the name of the currently selected option (if it's an option).
    fn selected_option_name(&self) -> Option<String> {
        let cursor = self.nav.current().cursor;
        match self.current_children().get(cursor) {
            Some(MenuChild::Option { name }) => Some(name.clone()),
            _ => None,
        }
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
        let Some(opt) = self.model_options.get(name) else {
            return;
        };
        let Some(choices) = opt.choices.as_ref() else {
            return;
        };
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

        self.values
            .insert(name.into(), ConfigValue::Choice(choices[new_idx].clone()));
        self.dirty = true;
    }

    /// Apply select and dependency propagation until stable.
    fn apply_selects(&mut self) {
        loop {
            let mut changed = false;

            // Pass 1: disable any enabled option whose depends_on is unsatisfied.
            for (name, opt) in &self.model_options {
                let is_enabled = matches!(self.values.get(name), Some(ConfigValue::Bool(true)));
                if is_enabled {
                    let deps_ok = opt
                        .depends_on
                        .iter()
                        .all(|dep| matches!(self.values.get(dep), Some(ConfigValue::Bool(true))));
                    if !deps_ok {
                        self.values.insert(name.clone(), ConfigValue::Bool(false));
                        changed = true;
                    }
                }
            }

            // Pass 2: enable any option forced on by another's selects.
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

    /// Save current values to .hadron-config.
    fn save(&mut self) -> Result<()> {
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

    /// Move cursor up within the current page (wraps around).
    fn cursor_up(&mut self) {
        let len = self.current_children().len();
        if len == 0 {
            return;
        }
        let page = self.nav.current_mut();
        if page.cursor > 0 {
            page.cursor -= 1;
        } else {
            page.cursor = len - 1;
        }
    }

    /// Move cursor down within the current page (wraps around).
    fn cursor_down(&mut self) {
        let len = self.current_children().len();
        if len == 0 {
            return;
        }
        let page = self.nav.current_mut();
        if page.cursor + 1 < len {
            page.cursor += 1;
        } else {
            page.cursor = 0;
        }
    }

    /// Enter a submenu (if the cursor is on one).
    fn enter_submenu(&mut self) {
        let cursor = self.nav.current().cursor;
        if let Some(MenuChild::SubMenu(_)) = self.current_children().get(cursor) {
            self.nav.push(cursor);
        }
    }

    /// Go back to the parent page.
    fn go_back(&mut self) {
        self.nav.pop();
    }

    /// Start editing the currently selected option.
    fn start_edit(&mut self) {
        let Some(name) = self.selected_option_name() else {
            return;
        };
        if !self.deps_satisfied(&name) {
            return;
        }
        let ty = self.model_options.get(&name).map(|o| o.ty);

        match ty {
            Some(ConfigType::Bool) => {
                self.toggle_bool(&name);
            }
            Some(ConfigType::Choice) => {
                self.cycle_choice(&name, true);
            }
            Some(ConfigType::Group) => {}
            Some(ConfigType::List) => {
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
                let current = self
                    .values
                    .get(&name)
                    .map(format_value)
                    .unwrap_or_default();
                self.editing = Some(EditState {
                    option_name: name,
                    buffer: current,
                });
            }
            None => {}
        }
    }

    /// Commit the current edit.
    fn commit_edit(&mut self) {
        let Some(edit) = self.editing.take() else {
            return;
        };
        let Some(opt) = self.model_options.get(&edit.option_name) else {
            return;
        };

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
            ConfigType::Bool | ConfigType::Group => {
                unreachable!("bools/groups are not text-edited")
            }
        };

        if let Some(val) = new_val {
            self.values.insert(edit.option_name, val);
            self.dirty = true;
        }
    }

    /// Open the help popup for the currently selected option.
    fn open_help(&mut self) {
        if let Some(name) = self.selected_option_name() {
            self.help_popup = Some(HelpPopupState {
                option_name: name,
                scroll_offset: 0,
            });
        }
    }

    /// Build help popup content lines for a given option.
    fn help_content(&self, name: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let opt = self.model_options.get(name);

        // Help text.
        if let Some(opt) = opt {
            lines.push(format!("  {} ", name));
            lines.push(String::new());
            if let Some(help) = &opt.help {
                lines.push(help.clone());
            } else {
                lines.push("No help available.".into());
            }
            lines.push(String::new());

            // Type / default / range / current value.
            lines.push(format!("  Type:    {:?}", opt.ty));
            lines.push(format!("  Default: {}", format_value(&opt.default)));
            if let Some(val) = self.values.get(name) {
                lines.push(format!("  Current: {}", format_value(val)));
            }
            if let Some((lo, hi)) = opt.range {
                lines.push(format!("  Range:   {lo}..{hi}"));
            }
            if let Some(choices) = &opt.choices {
                lines.push(format!("  Choices: {}", choices.join(", ")));
            }

            // Dependencies.
            if !opt.depends_on.is_empty() {
                lines.push(String::new());
                lines.push(format!("  Depends on: {}", opt.depends_on.join(", ")));
            }
            if !opt.selects.is_empty() {
                lines.push(format!("  Selects:    {}", opt.selects.join(", ")));
            }
        }

        // Reverse deps.
        if let Some(required_by) = self.reverse_deps.depended_on_by.get(name) {
            lines.push(String::new());
            lines.push(format!("  Required by: {}", required_by.join(", ")));
        }
        if let Some(selected_by) = self.reverse_deps.selected_by.get(name) {
            lines.push(format!("  Selected by: {}", selected_by.join(", ")));
        }

        // Impact analysis: what would be disabled if this is turned off.
        if matches!(self.values.get(name), Some(ConfigValue::Bool(true))) {
            let mut impacted = Vec::new();
            for (other_name, other_opt) in &self.model_options {
                if other_name == name {
                    continue;
                }
                if other_opt.depends_on.contains(&name.to_string()) {
                    if matches!(self.values.get(other_name), Some(ConfigValue::Bool(true))) {
                        impacted.push(other_name.clone());
                    }
                }
            }
            if !impacted.is_empty() {
                lines.push(String::new());
                lines.push(format!(
                    "  Would disable if turned off: {}",
                    impacted.join(", ")
                ));
            }
        }

        lines
    }

    /// Execute a search and populate search_results.
    fn execute_search(&mut self) {
        self.search_results.clear();
        self.search_cursor = 0;
        if !self.search_query.is_empty() {
            search_tree(
                &self.menu_tree,
                &self.search_query,
                &self.model_options,
                &[],
                &mut self.search_results,
            );
        }
    }

    /// Navigate to the page containing a search result.
    fn navigate_to_search_result(&mut self) {
        if let Some(result) = self.search_results.get(self.search_cursor) {
            // Rebuild the nav stack to the result's path.
            self.nav = NavStack::new();
            for &idx in &result.path {
                self.nav.push(idx);
            }
            self.nav.current_mut().cursor = result.child_index;
            self.search_mode = false;
            self.search_query.clear();
            self.search_results.clear();
        }
    }
}

// ─── Entry point ─────────────────────────────────────────────────────

/// Run the interactive TUI menuconfig.
pub fn run_menuconfig(
    model: &BuildModel,
    root: &Path,
    profile_name: &str,
    kconfig_tree: &crate::kconfig::ast::KconfigFile,
) -> Result<()> {
    let mut app = App::new(model, root, profile_name, kconfig_tree)?;

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

// ─── Event loop ──────────────────────────────────────────────────────

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw_ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            // Priority 1: Help popup.
            if app.help_popup.is_some() {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                        app.help_popup = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(popup) = &mut app.help_popup {
                            popup.scroll_offset = popup.scroll_offset.saturating_sub(1);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(popup) = &mut app.help_popup {
                            popup.scroll_offset += 1;
                        }
                    }
                    _ => {}
                }
                continue;
            }

            // Priority 2: Editing mode.
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

            // Priority 3: Search mode.
            if app.search_mode {
                match key.code {
                    KeyCode::Enter => {
                        app.navigate_to_search_result();
                    }
                    KeyCode::Esc => {
                        app.search_mode = false;
                        app.search_query.clear();
                        app.search_results.clear();
                    }
                    KeyCode::Backspace => {
                        app.search_query.pop();
                        app.execute_search();
                    }
                    KeyCode::Char(c) => {
                        app.search_query.push(c);
                        app.execute_search();
                    }
                    KeyCode::Up => {
                        if !app.search_results.is_empty() {
                            if app.search_cursor > 0 {
                                app.search_cursor -= 1;
                            } else {
                                app.search_cursor = app.search_results.len() - 1;
                            }
                        }
                    }
                    KeyCode::Down => {
                        if !app.search_results.is_empty() {
                            if app.search_cursor + 1 < app.search_results.len() {
                                app.search_cursor += 1;
                            } else {
                                app.search_cursor = 0;
                            }
                        }
                    }
                    _ => {}
                }
                continue;
            }

            // Priority 4: Normal navigation.
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    if app.dirty {
                        app.save()?;
                    }
                    return Ok(());
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Up | KeyCode::Char('k') => app.cursor_up(),
                KeyCode::Down | KeyCode::Char('j') => app.cursor_down(),
                KeyCode::Enter | KeyCode::Right => {
                    let cursor = app.nav.current().cursor;
                    match app.current_children().get(cursor) {
                        Some(MenuChild::SubMenu(_)) => app.enter_submenu(),
                        Some(MenuChild::Option { .. }) => app.start_edit(),
                        _ => {}
                    }
                }
                KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => {
                    app.go_back();
                }
                KeyCode::Char(' ') => {
                    // Space toggles/cycles the current option.
                    if let Some(name) = app.selected_option_name() {
                        let ty = app.model_options.get(&name).map(|o| o.ty);
                        match ty {
                            Some(ConfigType::Choice) => app.cycle_choice(&name, true),
                            _ => app.toggle_bool(&name),
                        }
                    }
                }
                KeyCode::Char('?') => app.open_help(),
                KeyCode::Char('/') => {
                    app.search_mode = true;
                    app.search_query.clear();
                    app.search_results.clear();
                    app.search_cursor = 0;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    app.save()?;
                }
                _ => {}
            }
        }
    }
}

// ─── Rendering ───────────────────────────────────────────────────────

fn draw_ui(f: &mut ratatui::Frame, app: &mut App) {
    let size = f.area();

    // Layout: breadcrumb (1), page list (min 6), status bar (1), key hints (3).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(size);

    // Search mode replaces the page list.
    if app.search_mode {
        draw_search_ui(f, app, &chunks);
    } else {
        draw_breadcrumb(f, app, chunks[0]);
        draw_page_list(f, app, chunks[1]);
    }
    draw_status_bar(f, app, chunks[2]);
    draw_key_hints(f, app, chunks[3]);

    // Help popup overlay.
    if let Some(popup) = &app.help_popup {
        draw_help_popup(f, app, popup, size);
    }

    // Editing overlay on status bar.
    if let Some(ref edit) = app.editing {
        let text = format!(" Editing {}: {}█", edit.option_name, edit.buffer);
        let widget = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
        f.render_widget(widget, chunks[2]);
    }
}

fn draw_breadcrumb(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let crumbs = app.nav.breadcrumbs(&app.menu_tree);
    let breadcrumb = crumbs.join(" > ");
    let widget = Paragraph::new(format!(" {breadcrumb}"))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(widget, area);
}

fn draw_page_list(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    // Adjust scroll offset first (mutable borrow).
    let list_height = area.height.saturating_sub(2) as usize;
    {
        let page = app.nav.current_mut();
        if page.cursor < page.scroll_offset {
            page.scroll_offset = page.cursor;
        }
        if page.cursor >= page.scroll_offset + list_height && list_height > 0 {
            page.scroll_offset = page.cursor.saturating_sub(list_height - 1);
        }
    }

    let children = app.current_children();
    let cursor = app.nav.current().cursor;
    let scroll_offset = app.nav.current().scroll_offset;

    let items: Vec<ListItem> = children
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(list_height)
        .map(|(i, child)| {
            let is_selected = i == cursor;
            match child {
                MenuChild::SubMenu(sub) => {
                    let count = sub.children.len();
                    let style = Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD);
                    let style = if is_selected {
                        style.bg(Color::DarkGray)
                    } else {
                        style
                    };
                    ListItem::new(Line::from(Span::styled(
                        format!("  ▶ {}  ({count} items)", sub.title),
                        style,
                    )))
                }
                MenuChild::Option { name } => {
                    let opt = app.model_options.get(name);
                    let val = app.values.get(name);
                    let deps_ok = app.deps_satisfied(name);

                    let val_str = match (opt.map(|o| o.ty), val) {
                        (Some(ConfigType::Bool), Some(ConfigValue::Bool(true))) => {
                            "[*]".to_string()
                        }
                        (Some(ConfigType::Bool), _) => "[ ]".to_string(),
                        (Some(ConfigType::Choice), Some(ConfigValue::Choice(v))) => {
                            format!("< {v} >")
                        }
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

                    let line = format!("      {val_str} {name:<24} {help_brief}");
                    ListItem::new(Line::from(Span::styled(line, style)))
                }
            }
        })
        .collect();

    let title = if app.dirty {
        " Configuration [modified] "
    } else {
        " Configuration "
    };
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, area);
}

fn draw_search_ui(f: &mut ratatui::Frame, app: &App, chunks: &[Rect]) {
    // Breadcrumb area shows search prompt.
    let search_text = format!(" / {}_", app.search_query);
    let search_widget = Paragraph::new(search_text)
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_widget(search_widget, chunks[0]);

    // Page list area shows search results.
    let list_height = chunks[1].height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .search_results
        .iter()
        .enumerate()
        .take(list_height)
        .map(|(i, result)| {
            let is_selected = i == app.search_cursor;
            let help_brief = app
                .model_options
                .get(&result.name)
                .and_then(|o| o.help.as_ref())
                .map(|h| h.as_str())
                .unwrap_or("");

            let style = if is_selected {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(Span::styled(
                format!("      {:<24} {help_brief}", result.name),
                style,
            )))
        })
        .collect();

    let title = format!(" Search: {} result(s) ", app.search_results.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, chunks[1]);
}

fn draw_status_bar(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let children = app.current_children();
    let cursor = app.nav.current().cursor;
    let total = children.len();
    let pos = if total > 0 { cursor + 1 } else { 0 };

    let status = match children.get(cursor) {
        Some(MenuChild::Option { name }) => {
            let opt = app.model_options.get(name);
            let type_str = opt.map(|o| format!("{:?}", o.ty)).unwrap_or_default();
            format!(" {name} ({type_str})  [{pos}/{total}]")
        }
        Some(MenuChild::SubMenu(sub)) => {
            format!(" {} (submenu)  [{pos}/{total}]", sub.title)
        }
        None => format!(" (empty)  [{pos}/{total}]"),
    };

    let widget =
        Paragraph::new(status).style(Style::default().fg(Color::Black).bg(Color::DarkGray));
    f.render_widget(widget, area);
}

fn draw_key_hints(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let hints = if app.editing.is_some() {
        " Enter Confirm  Esc Cancel "
    } else if app.search_mode {
        " ↑↓/jk Navigate  Enter Select  Esc Cancel "
    } else if app.help_popup.is_some() {
        " ↑↓/jk Scroll  Esc/? Close "
    } else {
        " ↑↓/jk Navigate  Enter/→ Into  Esc/← Back  Space Toggle  ? Help  / Search  S Save  Q Quit "
    };
    let widget = Paragraph::new(hints).block(Block::default().borders(Borders::ALL));
    f.render_widget(widget, area);
}

fn draw_help_popup(
    f: &mut ratatui::Frame,
    app: &App,
    popup: &HelpPopupState,
    area: Rect,
) {
    let content_lines = app.help_content(&popup.option_name);

    // Center the popup with ~60% width and ~60% height.
    let popup_width = (area.width * 3 / 5).max(40).min(area.width.saturating_sub(4));
    let popup_height = (area.height * 3 / 5).max(10).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the area behind the popup.
    f.render_widget(Clear, popup_area);

    let visible_height = popup_height.saturating_sub(2) as usize;
    let text: String = content_lines
        .iter()
        .skip(popup.scroll_offset)
        .take(visible_height)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    let title = format!(" Help: {} ", popup.option_name);
    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().bg(Color::Black)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, popup_area);
}

// ─── Helpers ─────────────────────────────────────────────────────────

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
