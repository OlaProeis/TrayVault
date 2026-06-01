//! Custom window chrome: search field (caption-drag + click-to-focus), settings icon, and close button.

use crate::app::App;
use crate::ui::pixmap::Pixmap;
use crate::ui::search_edit::{
    has_selection, search_caret_pixel_x, search_scroll_x, search_text_inner_width, selection_range,
    SEARCH_TEXT_SIZE_PX,
};
use crate::ui::theme::Theme;
use crate::ui::widgets::{fill_rect, input_box, rgba_to_color, UiContext, WidgetRect, PADDING};
use crate::ui::UiState;

/// Height of the custom title bar (replaces the native Windows caption).
pub const TITLE_BAR_HEIGHT: f32 = 28.0;

pub const TITLE_BUTTON_WIDTH: f32 = 28.0;

const SEARCH_INPUT_HEIGHT: f32 = 22.0;
const SEARCH_TEXT_Y_OFFSET: f32 = 3.5;
const SEARCH_PLACEHOLDER: &str = "Search .. (Doubble Click)";

/// Layout of the search field in the title bar (matches [`draw_title_bar`]).
pub fn search_input_rect(width: f32) -> WidgetRect {
    let right_buttons = TITLE_BUTTON_WIDTH * 2.0;
    let input_w = (width - PADDING - right_buttons - 4.0).max(60.0);
    WidgetRect {
        x: PADDING,
        y: (TITLE_BAR_HEIGHT - SEARCH_INPUT_HEIGHT) / 2.0,
        w: input_w,
        h: SEARCH_INPUT_HEIGHT,
    }
}

pub fn settings_button_rect(width: f32) -> WidgetRect {
    WidgetRect {
        x: width - TITLE_BUTTON_WIDTH * 2.0,
        y: 0.0,
        w: TITLE_BUTTON_WIDTH,
        h: TITLE_BAR_HEIGHT,
    }
}

pub fn close_button_rect(width: f32) -> WidgetRect {
    WidgetRect {
        x: width - TITLE_BUTTON_WIDTH,
        y: 0.0,
        w: TITLE_BUTTON_WIDTH,
        h: TITLE_BAR_HEIGHT,
    }
}

/// True when `(cx, cy)` in client coords is over the search field.
pub fn search_input_is_hit(cx: f32, cy: f32, width: f32) -> bool {
    if cy >= TITLE_BAR_HEIGHT {
        return false;
    }
    search_input_rect(width).contains(cx, cy)
}

/// True when `(cx, cy)` in client coords is over interactive title-bar chrome (not drag).
///
/// The search field is excluded so `WM_NCHITTEST` returns `HTCAPTION` there — Windows
/// handles drag natively. Clicks without movement are routed via `WM_NCLBUTTONUP` in
/// [`window.rs`](../../win32/window.rs).
pub fn title_bar_is_client_hit(cx: f32, cy: f32, width: f32) -> bool {
    if cy >= TITLE_BAR_HEIGHT {
        return false;
    }
    settings_button_rect(width).contains(cx, cy) || close_button_rect(width).contains(cx, cy)
}

fn draw_search_caret(
    pixmap: &mut Pixmap,
    cache: &mut crate::ui::text::GlyphCache,
    query: &str,
    caret: usize,
    rect: WidgetRect,
    scroll_x: f32,
    theme: &Theme,
) {
    let caret_x = search_caret_pixel_x(cache, query, rect, caret, scroll_x);
    let top = rect.y + 5.0;
    let h = rect.h - 10.0;
    fill_rect(pixmap, caret_x, top, 1.5, h, rgba_to_color(theme.accent));
}

#[allow(clippy::too_many_arguments)]
fn draw_icon_button(
    pixmap: &mut Pixmap,
    ctx: &mut UiContext<'_>,
    rect: WidgetRect,
    hovered: bool,
    hover_bg: [u8; 4],
    normal_bg: [u8; 4],
    icon: &str,
    icon_size: f32,
    icon_color: [u8; 4],
) {
    let bg = if hovered { hover_bg } else { normal_bg };
    fill_rect(pixmap, rect.x, rect.y, rect.w, rect.h, rgba_to_color(bg));

    let glyph_w = ctx.cache.measure(icon, icon_size);
    ctx.cache.draw(
        pixmap,
        icon,
        rect.x + (rect.w - glyph_w) / 2.0,
        rect.y + rect.h * 0.72,
        icon_size,
        rgba_to_color(icon_color),
    );
}

/// 4-tooth gear silhouette drawn with plain fill_rects — no font glyphs needed.
///
/// An octagonal body (7×7, corners trimmed) is flanked by four axis-aligned
/// teeth at N / S / E / W.  `bg` is painted over the centre to punch a small
/// cross-shaped hole so the gear reads instantly at 28 px.
fn draw_settings_icon(pixmap: &mut Pixmap, rect: WidgetRect, color: [u8; 4], bg: [u8; 4]) {
    let c = rgba_to_color(color);
    let b = rgba_to_color(bg);
    let cx = (rect.x + rect.w / 2.0).floor();
    let cy = (rect.y + rect.h / 2.0).floor();

    // Octagonal body: three overlapping rects give a 7×7 disc with cut corners.
    fill_rect(pixmap, cx - 3.0, cy - 1.0, 7.0, 3.0, c); // horizontal band
    fill_rect(pixmap, cx - 1.0, cy - 3.0, 3.0, 7.0, c); // vertical band
    fill_rect(pixmap, cx - 2.0, cy - 2.0, 5.0, 5.0, c); // interior

    // 4 axis-aligned teeth (3 wide × 2 deep), one pixel away from the body edge.
    fill_rect(pixmap, cx - 1.0, cy - 5.0, 3.0, 2.0, c); // top
    fill_rect(pixmap, cx - 1.0, cy + 4.0, 3.0, 2.0, c); // bottom
    fill_rect(pixmap, cx - 5.0, cy - 1.0, 2.0, 3.0, c); // left
    fill_rect(pixmap, cx + 4.0, cy - 1.0, 2.0, 3.0, c); // right

    // Centre hole: cross pattern approximates a small circle.
    fill_rect(pixmap, cx - 1.0, cy, 3.0, 1.0, b);
    fill_rect(pixmap, cx, cy - 1.0, 1.0, 3.0, b);
}

/// Draw title bar background, search field, settings icon, and close button.
pub fn draw_title_bar(
    pixmap: &mut Pixmap,
    theme: &Theme,
    app: &App,
    ui: &mut UiState,
    ctx: &mut UiContext<'_>,
    width: f32,
) {
    fill_rect(
        pixmap,
        0.0,
        0.0,
        width,
        TITLE_BAR_HEIGHT,
        rgba_to_color(theme.card),
    );
    fill_rect(
        pixmap,
        0.0,
        TITLE_BAR_HEIGHT - 1.0,
        width,
        1.0,
        rgba_to_color(theme.divider),
    );

    if ui.show_settings {
        ui.search_input_rect = None;

        let close_rect = close_button_rect(width);
        ui.close_button_rect = Some(close_rect);
        let close_hovered = close_rect.contains(ctx.mouse_x, ctx.mouse_y);
        if close_hovered {
            ctx.hot_widget = ctx.active_widget.max(1);
        }
        let close_icon_color = if close_hovered {
            [0xFF, 0xFF, 0xFF, 0xFF]
        } else {
            theme.text_secondary
        };
        draw_icon_button(
            pixmap,
            ctx,
            close_rect,
            close_hovered,
            [0xC4, 0x2B, 0x1C, 0xFF],
            theme.card,
            "\u{00D7}",
            16.0,
            close_icon_color,
        );

        let settings_rect = settings_button_rect(width);
        ui.settings_button_rect = Some(settings_rect);
        let settings_hovered = settings_rect.contains(ctx.mouse_x, ctx.mouse_y);
        if settings_hovered {
            ctx.hot_widget = ctx.active_widget.max(1);
        }
        let settings_bg = if settings_hovered {
            theme.selection
        } else {
            theme.accent
        };
        fill_rect(
            pixmap,
            settings_rect.x,
            settings_rect.y,
            settings_rect.w,
            settings_rect.h,
            rgba_to_color(settings_bg),
        );
        draw_settings_icon(pixmap, settings_rect, theme.text_primary, settings_bg);
        return;
    }

    let search_rect = search_input_rect(width);
    let inner_w = search_text_inner_width(search_rect);
    let scroll_x = search_scroll_x(ctx.cache, &app.filter_query, inner_w, ui.search_caret);
    let selection = if has_selection(ui.search_sel_anchor, ui.search_caret) {
        Some(selection_range(ui.search_sel_anchor, ui.search_caret))
    } else {
        None
    };
    let input = input_box(
        ctx,
        pixmap,
        &app.filter_query,
        SEARCH_PLACEHOLDER,
        search_rect.x,
        search_rect.y,
        search_rect.w,
        search_rect.h,
        ui.search_focused,
        SEARCH_TEXT_SIZE_PX,
        scroll_x,
        selection,
        SEARCH_TEXT_Y_OFFSET,
    );
    ui.search_input_rect = Some(search_rect);
    if input.focused {
        ui.search_focused = true;
    }
    if ui.search_focused {
        ui.search_caret = ui.search_caret.min(app.filter_query.len());
        ui.search_sel_anchor = ui.search_sel_anchor.min(app.filter_query.len());
        draw_search_caret(
            pixmap,
            ctx.cache,
            &app.filter_query,
            ui.search_caret,
            search_rect,
            scroll_x,
            theme,
        );
    }

    let settings_rect = settings_button_rect(width);
    ui.settings_button_rect = Some(settings_rect);
    let settings_hovered = settings_rect.contains(ctx.mouse_x, ctx.mouse_y);
    if settings_hovered {
        ctx.hot_widget = ctx.active_widget.max(1);
    }
    let settings_bg = if settings_hovered {
        theme.selection
    } else {
        theme.card
    };
    fill_rect(
        pixmap,
        settings_rect.x,
        settings_rect.y,
        settings_rect.w,
        settings_rect.h,
        rgba_to_color(settings_bg),
    );
    let settings_icon_color = if settings_hovered {
        theme.text_primary
    } else {
        theme.text_secondary
    };
    draw_settings_icon(pixmap, settings_rect, settings_icon_color, settings_bg);

    let close_rect = close_button_rect(width);
    ui.close_button_rect = Some(close_rect);
    let close_hovered = close_rect.contains(ctx.mouse_x, ctx.mouse_y);
    if close_hovered {
        ctx.hot_widget = ctx.active_widget.max(1);
    }
    let close_icon_color = if close_hovered {
        [0xFF, 0xFF, 0xFF, 0xFF]
    } else {
        theme.text_secondary
    };
    draw_icon_button(
        pixmap,
        ctx,
        close_rect,
        close_hovered,
        [0xC4, 0x2B, 0x1C, 0xFF],
        theme.card,
        "\u{00D7}",
        16.0,
        close_icon_color,
    );
}
