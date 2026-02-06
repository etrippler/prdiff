use crate::app::App;
use crate::logging;
use crate::model::{DiffSource, FileEntry, HighlightedLine, TreeNode};
use crate::theme::Theme;
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
        KeyModifiers, KeyboardEnhancementFlags, MouseEvent, MouseEventKind,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use std::io::{stdout, Write, Stdout};
use std::process::Command;
use std::time::Duration;

pub fn new_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    Ok(Terminal::new(CrosstermBackend::new(stdout()))?)
}

#[derive(Clone, Copy)]
struct UiLayout {
    tree_area: Rect,
    diff_area: Rect,
    tree_inner: Rect,
    diff_inner: Rect,
    help_area: Rect,
}

fn compute_layout(area: Rect, split_percent: u16) -> UiLayout {
    // Reserve the bottom row for the help footer before splitting panels
    let main_height = area.height.saturating_sub(1);
    let main_area = Rect::new(area.x, area.y, area.width, main_height);
    let help_area = Rect::new(area.x, area.y.saturating_add(main_height), area.width, 1);

    let tree_pct = split_percent.clamp(10, 90);
    let diff_pct = 100 - tree_pct;
    let chunks =
        Layout::horizontal([Constraint::Percentage(tree_pct), Constraint::Percentage(diff_pct)])
            .split(main_area);
    let tree_area = chunks[0];
    let diff_area = chunks[1];

    let tree_inner = Rect::new(
        tree_area.x.saturating_add(1),
        tree_area.y.saturating_add(1),
        tree_area.width.saturating_sub(2),
        tree_area.height.saturating_sub(2),
    );
    let diff_inner = Rect::new(
        diff_area.x.saturating_add(1),
        diff_area.y.saturating_add(1),
        diff_area.width.saturating_sub(2),
        diff_area.height.saturating_sub(2),
    );

    UiLayout {
        tree_area,
        diff_area,
        tree_inner,
        diff_inner,
        help_area,
    }
}

pub fn run_app(app: &mut App, terminal: &mut Terminal<impl Backend>, guard: &mut TerminalGuard) -> Result<()> {
    let mut needs_redraw = true;

    // Cache for visible items - only rebuild when tree changes
    let mut cached_visible: Vec<(usize, String, bool, Option<FileEntry>)> = Vec::new();
    let mut last_tree_version = 0u64;

    loop {
        // === PHASE 1: Handle ALL pending events first (responsive input) ===
        let mut had_events = false;
        while event::poll(Duration::from_millis(0))? {
            had_events = true;
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // Ctrl+C always exits
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        return Ok(());
                    }

                    // NOTE: Esc is NOT used as exit key because some terminals/escape sequences
                    // can be misinterpreted as Esc, causing unexpected exits. Use 'q' or Ctrl+C.

                    if app.branch_modal.is_some() {
                        handle_modal_key(app, key.code, key.modifiers);
                        needs_redraw = true;
                    } else {
                        // Get layout for key handling
                        let term_size = terminal.size()?;
                        let layout =
                            compute_layout(Rect::new(0, 0, term_size.width, term_size.height), app.split_percent);

                        match handle_key(app, key.code, &layout, &cached_visible) {
                            KeyAction::Quit => return Ok(()),
                            KeyAction::OpenEditor => {
                                if let Some((editor, path)) = app.editor_command() {
                                    guard.restore();
                                    let _ = Command::new(&editor).arg(&path).status();
                                    guard.enter()?;
                                    terminal.clear()?;
                                }
                            }
                            KeyAction::Continue => {}
                        }
                        needs_redraw = true;
                    }
                }
                Event::Mouse(mouse) => {
                    if app.branch_modal.is_none() {
                        let term_size = terminal.size()?;
                        let layout =
                            compute_layout(Rect::new(0, 0, term_size.width, term_size.height), app.split_percent);
                        handle_mouse(app, &layout, &mouse, cached_visible.len());
                        needs_redraw = true;
                    }
                }
                Event::Resize(_, _) => {
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        // === PHASE 2: Check for file changes (throttled internally) ===
        app.check_for_changes();

        // === PHASE 3: Rebuild visible items cache if tree changed ===
        let tree_version = app.tree_version();
        if tree_version != last_tree_version {
            cached_visible = app
                .visible_items()
                .into_iter()
                .map(|(depth, path, node)| {
                    let is_dir = matches!(node, TreeNode::Directory { .. });
                    let file = if let TreeNode::File(f) = node {
                        Some(f.clone())
                    } else {
                        None
                    };
                    (depth, path, is_dir, file)
                })
                .collect();
            last_tree_version = tree_version;
            needs_redraw = true;
        }

        // Defensive: keep cursor stable if the tree shrinks.
        if !cached_visible.is_empty() && app.cursor >= cached_visible.len() {
            app.cursor = cached_visible.len() - 1;
            app.diff_scroll = 0;
            needs_redraw = true;
        }
        if cached_visible.is_empty() {
            app.cursor = 0;
            app.scroll_offset = 0;
            app.diff_scroll = 0;
        }

        // === PHASE 4: Render (only if needed) ===
        if needs_redraw {
            let selected_file_path = cached_visible.get(app.cursor).and_then(|(_, _, is_dir, file)| {
                if !is_dir {
                    file.as_ref().map(|f| f.path.clone())
                } else {
                    None
                }
            });

            if let Some(ref path) = selected_file_path {
                app.ensure_highlighted(path);
                app.diff_line_count = app.get_highlighted(path).len();
            } else {
                app.diff_line_count = 0;
            }

            let highlighted_lines: &[HighlightedLine] = selected_file_path
                .as_ref()
                .map(|p| app.get_highlighted(p))
                .unwrap_or(&[]);
            let selected_diff_source = selected_file_path
                .as_ref()
                .and_then(|p| app.get_diff_source(p))
                .unwrap_or(DiffSource::Worktree);
            let selected_file_path_ref = selected_file_path.as_deref();
            let theme = &app.theme;
            let split_percent = app.split_percent;
            let base_branch = app.base_branch.as_str();
            let merge_base_short: String = app.merge_base.chars().take(7).collect();
            let expanded = &app.expanded;

            // Compute layout inside draw to use the authoritative frame area,
            // and clamp scroll values against that same layout.
            let mut draw_layout = None;
            let branch_modal = &app.branch_modal;
            let has_modal = branch_modal.is_some();
            terminal.draw(|f| {
                let layout = compute_layout(f.area(), split_percent);
                draw_layout = Some(layout);
                draw_ui(
                    f,
                    &layout,
                    &cached_visible,
                    app.cursor,
                    app.scroll_offset,
                    app.diff_scroll,
                    expanded,
                    base_branch,
                    &merge_base_short,
                    selected_file_path_ref,
                    selected_diff_source,
                    highlighted_lines,
                    theme,
                    has_modal,
                );
                if let Some(modal) = branch_modal {
                    draw_branch_modal(f, modal, base_branch, theme);
                }
            })?;

            if let Some(layout) = draw_layout {
                clamp_scroll(app, &layout);
                adjust_tree_scroll(app, &layout);
            }
            needs_redraw = false;
        }

        // === PHASE 5: Wait for next event (with timeout for file watching) ===
        if !had_events {
            // Short poll to stay responsive while allowing check_for_changes to run
            event::poll(Duration::from_millis(50))?;
        }
    }
}

fn clamp_scroll(app: &mut App, layout: &UiLayout) {
    let max_tree_visible = layout.tree_inner.height as usize;
    if max_tree_visible == 0 {
        app.scroll_offset = 0;
    }

    let max_diff_visible = layout.diff_inner.height as usize;
    let max_scroll = app.diff_line_count.saturating_sub(max_diff_visible);
    app.diff_scroll = app.diff_scroll.min(max_scroll);
}

fn adjust_tree_scroll(app: &mut App, layout: &UiLayout) {
    let max_tree_visible = layout.tree_inner.height as usize;
    if max_tree_visible == 0 {
        app.scroll_offset = 0;
        return;
    }

    if app.cursor >= app.scroll_offset.saturating_add(max_tree_visible) {
        // cursor should be the last visible row
        app.scroll_offset = app
            .cursor
            .saturating_add(1)
            .saturating_sub(max_tree_visible);
    }
    if app.cursor < app.scroll_offset {
        app.scroll_offset = app.cursor;
    }
}

enum KeyAction {
    Continue,
    Quit,
    OpenEditor,
}

fn handle_key(
    app: &mut App,
    code: KeyCode,
    layout: &UiLayout,
    visible: &[(usize, String, bool, Option<FileEntry>)],
) -> KeyAction {
    let visible_count = visible.len();
    match code {
        KeyCode::Char('q') => return KeyAction::Quit,
        KeyCode::Char('j') | KeyCode::Down => {
            if app.cursor < visible_count.saturating_sub(1) {
                app.cursor += 1;
                app.diff_scroll = 0;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.cursor > 0 {
                app.cursor -= 1;
                app.diff_scroll = 0;
            }
        }
        KeyCode::Char('J') => {
            let max_scroll = app
                .diff_line_count
                .saturating_sub(layout.diff_inner.height as usize);
            app.diff_scroll = app.diff_scroll.saturating_add(3).min(max_scroll);
        }
        KeyCode::Char('K') => {
            app.diff_scroll = app.diff_scroll.saturating_sub(3);
        }
        KeyCode::Char('h') | KeyCode::Left => {
            app.collapse_selected();
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if matches!(visible.get(app.cursor), Some((_, _, true, _))) {
                app.toggle_expand();
            }
        }
        KeyCode::Enter => {
            if matches!(visible.get(app.cursor), Some((_, _, true, _))) {
                app.toggle_expand();
            } else {
                return KeyAction::OpenEditor;
            }
        }
        KeyCode::Char(' ') => {
            if matches!(visible.get(app.cursor), Some((_, _, true, _))) {
                app.toggle_expand();
            }
        }
        KeyCode::Char('<') => {
            app.split_percent = app.split_percent.saturating_sub(5).max(10);
        }
        KeyCode::Char('>') => {
            app.split_percent = (app.split_percent + 5).min(90);
        }
        KeyCode::Char('b') => {
            app.open_branch_modal();
        }
        _ => {}
    }
    KeyAction::Continue
}

fn handle_modal_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let Some(modal) = &mut app.branch_modal else {
        return;
    };

    let ctrl = modifiers.contains(KeyModifiers::CONTROL);

    match code {
        KeyCode::Esc => {
            app.branch_modal = None;
        }
        KeyCode::Enter => {
            let selected = modal.selected_branch().map(|s| s.to_string());
            app.branch_modal = None;
            if let Some(branch) = selected {
                app.switch_base_branch(&branch);
            }
        }
        KeyCode::Up => {
            if modal.cursor > 0 {
                modal.cursor -= 1;
            }
        }
        KeyCode::Down => {
            if modal.cursor < modal.filtered.len().saturating_sub(1) {
                modal.cursor += 1;
            }
        }
        KeyCode::Char('k' | 'p') if ctrl => {
            if modal.cursor > 0 {
                modal.cursor -= 1;
            }
        }
        KeyCode::Char('j' | 'n') if ctrl => {
            if modal.cursor < modal.filtered.len().saturating_sub(1) {
                modal.cursor += 1;
            }
        }
        KeyCode::Backspace => {
            modal.query.pop();
            modal.update_filter();
        }
        KeyCode::Char(c) if !ctrl => {
            modal.query.push(c);
            modal.update_filter();
        }
        _ => {}
    }
}

fn handle_mouse(app: &mut App, layout: &UiLayout, mouse: &MouseEvent, visible_count: usize) {
    let x = mouse.column;
    let y = mouse.row;

    let in_diff_panel = x >= layout.diff_inner.x
        && x < layout.diff_inner.x.saturating_add(layout.diff_inner.width)
        && y >= layout.diff_inner.y
        && y < layout.diff_inner.y.saturating_add(layout.diff_inner.height);

    let in_tree_panel = x >= layout.tree_inner.x
        && x < layout.tree_inner.x.saturating_add(layout.tree_inner.width)
        && y >= layout.tree_inner.y
        && y < layout.tree_inner.y.saturating_add(layout.tree_inner.height);

    logging::trace_mouse(mouse, in_tree_panel, in_diff_panel);

    match mouse.kind {
        MouseEventKind::Down(_) => {
            if in_tree_panel {
                let clicked_row = y.saturating_sub(layout.tree_inner.y) as usize;
                let new_cursor = app.scroll_offset.saturating_add(clicked_row);
                if new_cursor < visible_count {
                    app.cursor = new_cursor;
                    app.diff_scroll = 0;
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if in_diff_panel {
                let max_scroll = app
                    .diff_line_count
                    .saturating_sub(layout.diff_inner.height as usize);
                app.diff_scroll = app.diff_scroll.saturating_add(3).min(max_scroll);
            }
        }
        MouseEventKind::ScrollUp => {
            if in_diff_panel {
                app.diff_scroll = app.diff_scroll.saturating_sub(3);
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ui(
    f: &mut Frame,
    layout: &UiLayout,
    visible: &[(usize, String, bool, Option<FileEntry>)],
    cursor: usize,
    scroll_offset: usize,
    diff_scroll: usize,
    expanded: &std::collections::HashSet<String>,
    base_branch: &str,
    merge_base_short: &str,
    selected_file_path: Option<&str>,
    selected_diff_source: DiffSource,
    highlighted_lines: &[HighlightedLine],
    theme: &Theme,
    has_modal: bool,
) {
    // File tree
    let tree_block = Block::default()
        .title(format!(
            " prdiff vs {base_branch} (merge-base {merge_base_short}) "
        ))
        .borders(Borders::ALL);
    let tree_inner = tree_block.inner(layout.tree_area);
    f.render_widget(tree_block, layout.tree_area);

    let max_tree_visible = tree_inner.height as usize;
    let mut lines: Vec<Line> = Vec::new();
    if visible.is_empty() {
        lines.push(Line::styled(
            "No changes",
            Style::default().fg(Color::DarkGray),
        ));
    }

    for (i, (depth, path, is_dir, file)) in visible
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(max_tree_visible)
    {
        let indent = "  ".repeat(*depth);
        let is_selected = i == cursor;

        let (prefix, name, style) = if *is_dir {
            let is_exp = expanded.contains(path);
            let arrow = if is_exp { "▼ " } else { "▶ " };
            let dir_name = path.rsplit('/').next().unwrap_or(path);
            (
                arrow.to_string(),
                format!("{dir_name}/"),
                Style::default().fg(Color::Blue).bold(),
            )
        } else if let Some(f) = file {
            let fname = f.path.rsplit('/').next().unwrap_or(&f.path);
            let stats = format!(" +{}/-{}", f.additions, f.deletions);
            (
                format!("{} ", f.status.symbol()),
                format!("{fname}{stats}"),
                Style::default().fg(f.status.color()),
            )
        } else {
            continue;
        };

        let line_style = if is_selected {
            Style::default()
                .bg(theme.selected_bg)
                .fg(theme.selected_fg)
                .bold()
        } else {
            style
        };

        lines.push(Line::from(vec![
            Span::styled(indent, line_style),
            Span::styled(prefix, line_style),
            Span::styled(name, line_style),
        ]));
    }

    f.render_widget(Paragraph::new(lines), tree_inner);

    if visible.len() > max_tree_visible {
        let mut scrollbar_state = ScrollbarState::new(visible.len()).position(scroll_offset);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            layout.tree_area,
            &mut scrollbar_state,
        );
    }

    // Diff preview
    let diff_title = match selected_diff_source {
        DiffSource::Worktree => " Diff (worktree) ",
        DiffSource::Index => " Diff (staged) ",
        DiffSource::Untracked => " Diff (untracked) ",
    };
    let diff_block = Block::default().title(diff_title).borders(Borders::ALL);
    let diff_inner = diff_block.inner(layout.diff_area);
    f.render_widget(diff_block, layout.diff_area);

    if selected_file_path.is_some() {
        let max_diff_visible = diff_inner.height as usize;
        let clamped_scroll =
            diff_scroll.min(highlighted_lines.len().saturating_sub(max_diff_visible));

        let visible_end = (clamped_scroll + max_diff_visible).min(highlighted_lines.len());
        let visible_lines = &highlighted_lines[clamped_scroll..visible_end];

        let diff_text: Vec<Line> = visible_lines
            .iter()
            .map(|hl| {
                let spans: Vec<Span> = hl
                    .spans
                    .iter()
                    .map(|(text, fg, bg)| {
                        Span::styled(text.clone(), Style::default().fg(*fg).bg(*bg))
                    })
                    .collect();

                Line::from(spans)
            })
            .collect();

        f.render_widget(Paragraph::new(diff_text), diff_inner);

        if highlighted_lines.len() > max_diff_visible {
            let mut scrollbar_state =
                ScrollbarState::new(highlighted_lines.len()).position(clamped_scroll);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                layout.diff_area,
                &mut scrollbar_state,
            );
        }
    } else if let Some((_, path, true, _)) = visible.get(cursor) {
        let text = format!("Directory: {path}\n\nPress Space/Enter/→ to expand/collapse");
        f.render_widget(Paragraph::new(text), diff_inner);
    }

    // Help footer (skip if terminal is too small).
    if f.area().height > 0 {
        let help = if has_modal {
            " ↑/↓:nav | Enter:select | Esc:cancel | type to filter "
        } else {
            " j/k:nav | h/l/Space:expand | Enter:open | J/K:scroll | </>:resize | b:branch | q:quit "
        };
        f.render_widget(
            Paragraph::new(help).style(Style::default().bg(Color::DarkGray)),
            layout.help_area,
        );
    }
}

fn draw_branch_modal(
    f: &mut Frame,
    modal: &crate::app::BranchModal,
    current_base: &str,
    theme: &Theme,
) {
    let area = f.area();
    let width = 60.min(area.width.saturating_sub(4));
    let height = (area.height * 60 / 100).max(5).min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, modal_area);

    let block = Block::default()
        .title(" Switch base branch ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(modal_area);
    f.render_widget(block, modal_area);

    if inner.height < 2 || inner.width < 4 {
        return;
    }

    // Search input row
    let search_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let search_text = format!(" > {}_", modal.query);
    f.render_widget(
        Paragraph::new(search_text).style(Style::default().fg(Color::Yellow)),
        search_area,
    );

    // Branch list below the search
    let list_height = inner.height.saturating_sub(1) as usize;
    let list_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1));

    // Adjust modal scroll_offset for the list
    let scroll_offset = if modal.cursor >= modal.scroll_offset + list_height {
        modal.cursor.saturating_add(1).saturating_sub(list_height)
    } else if modal.cursor < modal.scroll_offset {
        modal.cursor
    } else {
        modal.scroll_offset
    };

    let mut lines: Vec<Line> = Vec::new();
    for (vi, &branch_idx) in modal
        .filtered
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(list_height)
    {
        let branch = &modal.branches[branch_idx];
        let is_current = branch == current_base;
        let prefix = if is_current { "* " } else { "  " };
        let label = format!("{prefix}{branch}");
        let is_selected = vi == modal.cursor;

        let style = if is_selected {
            Style::default()
                .bg(theme.selected_bg)
                .fg(theme.selected_fg)
                .bold()
        } else if is_current {
            Style::default().fg(Color::Green)
        } else {
            Style::default()
        };

        lines.push(Line::styled(label, style));
    }

    if lines.is_empty() {
        lines.push(Line::styled(
            "  No matching branches",
            Style::default().fg(Color::DarkGray),
        ));
    }

    f.render_widget(Paragraph::new(lines), list_area);
}

pub struct TerminalGuard {
    stdout: Stdout,
    restored: bool,
}

impl TerminalGuard {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = stdout();
        stdout.execute(EnterAlternateScreen)?;
        stdout.execute(EnableMouseCapture)?;
        // Enable kitty keyboard protocol for unambiguous escape sequences
        stdout.execute(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        ))?;
        Ok(Self {
            stdout,
            restored: false,
        })
    }

    pub fn restore(&mut self) {
        if self.restored {
            return;
        }

        // 1. Pop keyboard enhancement flags
        let _ = self.stdout.execute(PopKeyboardEnhancementFlags);

        // 2. Tell terminal to stop sending mouse events
        let _ = self.stdout.execute(DisableMouseCapture);
        let _ = self.stdout.flush();

        // 3. Drain any pending input events (escape sequences already in buffer)
        while event::poll(Duration::from_millis(0)).unwrap_or(false) {
            let _ = event::read();
        }

        // 4. Leave alternate screen and restore terminal
        let _ = self.stdout.execute(LeaveAlternateScreen);
        let _ = self.stdout.flush();
        let _ = disable_raw_mode();
        self.restored = true;
    }

    /// Re-enter the TUI after a temporary restore (e.g., editor launch).
    pub fn enter(&mut self) -> Result<()> {
        enable_raw_mode()?;
        self.stdout.execute(EnterAlternateScreen)?;
        self.stdout.execute(EnableMouseCapture)?;
        self.stdout.execute(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        ))?;
        self.restored = false;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
