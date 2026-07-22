use std::path::Path;

use super::filter::{FILTER_ITEM_COUNT, FilterState};
use super::panels::{BudgetDisplay, BudgetRates};
use super::render::{ViewState, clamp_scroll, render_dashboard};
use crate::dashboard::load_data;

pub(crate) fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
    telemetry_dir: Option<&Path>,
    project_id: Option<String>,
    architect: &rexymcp_executor::config::ArchitectConfig,
) -> std::io::Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use std::time::Duration;

    let mut offset: u16 = 0;
    let mut follow = true;
    let mut spinner_tick: usize = 0;
    let mut filter_state = FilterState::default();
    let mut budget_display = BudgetDisplay::Dollars;
    // Track record count so we can re-enable follow whenever new content arrives,
    // regardless of whether the user previously scrolled away from the bottom.
    let mut prev_record_count: usize = 0;

    loop {
        spinner_tick = spinner_tick.wrapping_add(1);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let data = load_data(
            repo,
            session,
            telemetry_dir,
            project_id.as_deref(),
            architect,
        );
        // New records arrived — snap back to the bottom so the live feed is always
        // visible. This re-engages autoscroll even if the user previously scrolled up.
        if data.records.len() > prev_record_count {
            follow = true;
        }
        prev_record_count = data.records.len();
        let spinner_active = data.summary.ended.is_none() && data.error.is_none();
        let spinner = if spinner_active {
            Some(spinner_tick)
        } else {
            None
        };
        let state = ViewState {
            offset,
            follow,
            spinner,
            filter: filter_state.clone(),
            budget_display,
        };
        let mut total_wrapped = 0usize;
        terminal.draw(|frame| {
            total_wrapped = render_dashboard(frame, frame.area(), &data, now_ms, &state, rates);
        })?;
        offset = clamp_scroll(offset, total_wrapped);

        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if filter_state.open {
                match key.code {
                    KeyCode::Char('f') | KeyCode::Esc => filter_state.open = false,
                    KeyCode::Char('j') | KeyCode::Down => {
                        filter_state.cursor = (filter_state.cursor + 1) % FILTER_ITEM_COUNT;
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        filter_state.cursor =
                            (filter_state.cursor + FILTER_ITEM_COUNT - 1) % FILTER_ITEM_COUNT;
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        filter_state.filter.toggle(filter_state.cursor);
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('f') => {
                        filter_state.open = true;
                        filter_state.cursor = 0;
                    }
                    KeyCode::Char('b') => {
                        budget_display = match budget_display {
                            BudgetDisplay::Dollars => BudgetDisplay::Tokens,
                            BudgetDisplay::Tokens => BudgetDisplay::Dollars,
                        };
                    }
                    KeyCode::Up => {
                        follow = false;
                        offset = offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        follow = false;
                        offset = offset.saturating_add(1);
                    }
                    KeyCode::PageUp => {
                        follow = false;
                        offset = offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        follow = false;
                        offset = offset.saturating_add(10);
                    }
                    KeyCode::Home => {
                        follow = false;
                        offset = 0;
                    }
                    KeyCode::End => {
                        follow = true;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
