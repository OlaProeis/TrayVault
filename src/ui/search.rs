//! Filter chips and keyboard navigation helpers.

use crate::ui::pixmap::Pixmap;

use crate::app::App;
use crate::ui::theme::Theme;
use crate::ui::widgets::{fill_rect, filter_chip, rgba_to_color, PADDING};
use crate::ui::{build_display_indices, display_indices_key_matches, EntryFilter, UiState};

pub const FILTER_ROW_HEIGHT: f32 = 36.0;
const CHROME_PAD: f32 = 8.0;

/// Draw filter chips in the bottom chrome band.
pub fn draw_filter_row(
    pixmap: &mut Pixmap,
    ui: &UiState,
    ctx: &mut crate::ui::widgets::UiContext<'_>,
    _width: f32,
    y: f32,
) {
    let chip_y = y + 11.0;
    let mut chip_x = PADDING;
    for filter in EntryFilter::ALL {
        let selected = ui.filter == filter;
        let (rect, _clicked) = filter_chip(ctx, pixmap, filter.label(), chip_x, chip_y, selected);
        chip_x = rect.x + rect.w + 8.0;
    }
}

/// Hit-test filter chips (coordinates match [`draw_filter_row`]).
pub fn hit_test_filter_chip(
    cache: &mut crate::ui::text::GlyphCache,
    x: f32,
    y: f32,
    client_height: f32,
) -> Option<EntryFilter> {
    use crate::ui::widgets::{WidgetRect, PADDING};

    let chip_y = filter_row_y(client_height) + 11.0;
    let mut chip_x = PADDING;
    for filter in EntryFilter::ALL {
        let label = filter.label();
        let text_w = cache.measure(label, 12.0);
        let w = text_w + PADDING * 1.5;
        let h = 22.0;
        let rect = WidgetRect {
            x: chip_x,
            y: chip_y,
            w,
            h,
        };
        if rect.contains(x, y) {
            return Some(filter);
        }
        chip_x = rect.x + rect.w + 8.0;
    }
    None
}

pub fn refresh_display_indices(app: &App, ui: &mut UiState) {
    if display_indices_key_matches(app, ui) {
        return;
    }
    ui.display_indices = build_display_indices(&app.entries, ui.filter, &app.filter_query);
    ui.store_display_indices_key(app);
    ui.display_indices_rebuild_count += 1;
}

pub fn sync_selection_to_display(app: &mut App, ui: &UiState) {
    if ui.display_indices.is_empty() {
        app.selected_index = 0;
        return;
    }
    if !ui.display_indices.contains(&app.selected_index) {
        app.selected_index = ui.display_indices[0];
    }
}

pub fn move_selection(app: &mut App, ui: &UiState, delta: i32) {
    if ui.display_indices.is_empty() {
        return;
    }
    let current_pos = ui
        .display_indices
        .iter()
        .position(|&i| i == app.selected_index)
        .unwrap_or(0);
    let new_pos =
        (current_pos as i32 + delta).clamp(0, ui.display_indices.len() as i32 - 1) as usize;
    app.selected_index = ui.display_indices[new_pos];
}

pub fn header_total_height() -> f32 {
    crate::ui::titlebar::TITLE_BAR_HEIGHT + CHROME_PAD
}

pub fn footer_total_height() -> f32 {
    FILTER_ROW_HEIGHT + CHROME_PAD
}

pub fn content_viewport_height(client_height: f32) -> f32 {
    (client_height - header_total_height() - footer_total_height()).max(0.0)
}

fn filter_row_y(client_height: f32) -> f32 {
    client_height - footer_total_height()
}

/// Title bar redrawn on top of scrollable content.
pub fn draw_sticky_header(
    pixmap: &mut Pixmap,
    theme: &Theme,
    app: &App,
    ui: &mut UiState,
    ctx: &mut crate::ui::widgets::UiContext<'_>,
    width: f32,
) {
    let title_h = crate::ui::titlebar::TITLE_BAR_HEIGHT;
    fill_rect(
        pixmap,
        0.0,
        title_h,
        width,
        header_total_height() - title_h,
        rgba_to_color(theme.background),
    );
    crate::ui::titlebar::draw_title_bar(pixmap, theme, app, ui, ctx, width);
}

/// Filter chips redrawn on top of scrollable content at the bottom.
pub fn draw_sticky_footer(
    pixmap: &mut Pixmap,
    theme: &Theme,
    ui: &mut UiState,
    ctx: &mut crate::ui::widgets::UiContext<'_>,
    width: f32,
    client_height: f32,
) {
    let footer_y = filter_row_y(client_height);
    fill_rect(
        pixmap,
        0.0,
        footer_y,
        width,
        footer_total_height(),
        rgba_to_color(theme.background),
    );
    fill_rect(
        pixmap,
        0.0,
        footer_y,
        width,
        1.0,
        rgba_to_color(theme.divider),
    );
    draw_filter_row(pixmap, ui, ctx, width, footer_y);
}
