//! Settings panel wired to [`crate::config::Config`] and runtime behavior.

use crate::ui::pixmap::Pixmap;
use crate::ui::search_edit::{
    has_selection, search_caret_pixel_x, search_scroll_x, search_text_inner_width, selection_range,
    SEARCH_TEXT_SIZE_PX,
};
use crate::ui::text::GlyphCache;

use crate::config::{Config, ThemeMode};
use crate::ui::theme::Theme;
use crate::ui::widgets::{
    draw_toggle, fill_rect, filter_chip, input_box, rgba_to_color, UiContext, WidgetRect, PADDING,
};
use crate::ui::{format_max_image_edit, SettingsFocus, SettingsRects, UiState};

const ROW_GAP: f32 = 8.0;
const HEADER_H: f32 = 48.0;
const FOOTER_H: f32 = 36.0;
const TOGGLE_ROW_H: f32 = 50.0;
const INPUT_BLOCK_H: f32 = 72.0;
const INPUT_H: f32 = 32.0;
const THEME_ROW_H: f32 = 36.0;
const BACK_BTN: f32 = 32.0;
const SEPARATOR_TOP: f32 = 4.0;
const SEPARATOR_BOTTOM: f32 = 12.0;
const SEPARATOR_H: f32 = SEPARATOR_TOP + 1.0 + SEPARATOR_BOTTOM;
const SEPARATOR_COUNT: f32 = 5.0;
const INFO_BLOCK_H: f32 = 78.0;

/// Crate version from `Cargo.toml` (shown in settings About).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Public GitHub repository for source, issues, and releases.
pub const GITHUB_REPO_URL: &str = "https://github.com/OlaProeis/TrayVault";

const GITHUB_LINK_LABEL: &str = "github.com/OlaProeis/TrayVault";

/// Total scrollable content height for the settings list.
pub fn settings_content_height() -> f32 {
    TOGGLE_ROW_H * 7.0
        + INPUT_BLOCK_H * 3.0
        + THEME_ROW_H
        + INFO_BLOCK_H
        + ROW_GAP * 12.0
        + SEPARATOR_H * SEPARATOR_COUNT
        + 8.0
}

pub fn draw_settings(
    pixmap: &mut Pixmap,
    theme: &Theme,
    config: &Config,
    pause_capture: bool,
    ui: &mut UiState,
    ctx: &mut UiContext<'_>,
    top: f32,
) {
    let width = pixmap.width() as f32;
    let height = pixmap.height() as f32;

    let content_h = settings_content_height();
    let viewport_h = (height - top - HEADER_H - FOOTER_H).max(0.0);
    let max_scroll = (content_h - viewport_h).max(0.0);
    ui.settings_scroll = ui.settings_scroll.clamp(0.0, max_scroll);

    let row_w = width - PADDING * 2.0;
    let mut rects = SettingsRects::default();

    // Fixed header: back button + title (does not scroll).
    let header_y = top + 8.0;
    let back_x = PADDING;
    let back_y = header_y;
    let back_rect = WidgetRect {
        x: back_x,
        y: back_y,
        w: BACK_BTN,
        h: BACK_BTN,
    };
    let back_hovered = back_rect.contains(ctx.mouse_x, ctx.mouse_y);
    let back_bg = if back_hovered {
        theme.selection
    } else {
        theme.card
    };
    fill_rect(
        pixmap,
        back_x,
        back_y,
        BACK_BTN,
        BACK_BTN,
        rgba_to_color(back_bg),
    );
    draw_back_button(ctx, pixmap, back_rect, theme);
    rects.back = Some(back_rect);
    ctx.cache.draw(
        pixmap,
        "Settings",
        back_x + BACK_BTN + 8.0,
        header_y + 20.0,
        18.0,
        rgba_to_color(theme.text_primary),
    );

    let mut y = top + HEADER_H - ui.settings_scroll;

    rects.pause = Some(draw_toggle_setting(
        ctx,
        pixmap,
        theme,
        "Pause capture",
        Some("Stop recording new clipboard items"),
        PADDING,
        y,
        row_w,
        pause_capture,
    ));
    y += TOGGLE_ROW_H + ROW_GAP;

    let max_entries_focused = ui.settings_focus == SettingsFocus::MaxEntries;
    let (max_entries_rect, _) = draw_input_setting(
        ctx,
        pixmap,
        theme,
        "Max history entries",
        "Oldest unpinned items are removed above this limit",
        &ui.settings_edit_max_entries,
        PADDING,
        y,
        row_w,
        max_entries_focused,
        if max_entries_focused {
            ui.settings_caret
        } else {
            0
        },
        if max_entries_focused {
            ui.settings_sel_anchor
        } else {
            0
        },
    );
    rects.max_entries = Some(max_entries_rect);
    y += INPUT_BLOCK_H + ROW_GAP;

    rects.deduplicate = Some(draw_toggle_setting(
        ctx,
        pixmap,
        theme,
        "Deduplicate globally",
        Some("Skip capture when the same content is already in history"),
        PADDING,
        y,
        row_w,
        config.deduplicate_global,
    ));
    y += TOGGLE_ROW_H + ROW_GAP;
    y = draw_separator(pixmap, theme, PADDING, y, row_w);

    let hotkey_focused = ui.settings_focus == SettingsFocus::Hotkey;
    let (hotkey_rect, _) = draw_input_setting(
        ctx,
        pixmap,
        theme,
        "Global hotkey",
        "Keyboard shortcut to show or hide TrayVault",
        &ui.settings_edit_hotkey,
        PADDING,
        y,
        row_w,
        hotkey_focused,
        if hotkey_focused { ui.settings_caret } else { 0 },
        if hotkey_focused {
            ui.settings_sel_anchor
        } else {
            0
        },
    );
    rects.hotkey = Some(hotkey_rect);
    y += INPUT_BLOCK_H + ROW_GAP;

    ctx.cache.draw(
        pixmap,
        "Theme",
        PADDING,
        y + 20.0,
        13.0,
        rgba_to_color(theme.text_secondary),
    );
    let chip_y = y + 4.0;
    let mut chip_x = PADDING;
    for (mode, label) in [
        (ThemeMode::Light, "Light"),
        (ThemeMode::Dark, "Dark"),
        (ThemeMode::System, "System"),
    ] {
        let selected = config.theme == mode;
        let (rect, _) = filter_chip(ctx, pixmap, label, chip_x, chip_y, selected);
        match mode {
            ThemeMode::Light => rects.theme_light = Some(rect),
            ThemeMode::Dark => rects.theme_dark = Some(rect),
            ThemeMode::System => rects.theme_system = Some(rect),
        }
        chip_x = rect.x + rect.w + 8.0;
    }
    y += THEME_ROW_H + ROW_GAP;
    y = draw_separator(pixmap, theme, PADDING, y, row_w);

    rects.capture_images = Some(draw_toggle_setting(
        ctx,
        pixmap,
        theme,
        "Capture images",
        None,
        PADDING,
        y,
        row_w,
        config.capture_images,
    ));
    y += TOGGLE_ROW_H + ROW_GAP;

    rects.capture_rich_text = Some(draw_toggle_setting(
        ctx,
        pixmap,
        theme,
        "Capture rich text",
        None,
        PADDING,
        y,
        row_w,
        config.capture_rich_text,
    ));
    y += TOGGLE_ROW_H + ROW_GAP;

    rects.close_on_copy = Some(draw_toggle_setting(
        ctx,
        pixmap,
        theme,
        "Close on copy",
        Some("Hide the window after copying an entry back to the clipboard"),
        PADDING,
        y,
        row_w,
        config.close_on_copy,
    ));
    y += TOGGLE_ROW_H + ROW_GAP;

    rects.show_in_taskbar = Some(draw_toggle_setting(
        ctx,
        pixmap,
        theme,
        "Show in taskbar when open",
        None,
        PADDING,
        y,
        row_w,
        config.show_in_taskbar,
    ));
    y += TOGGLE_ROW_H + ROW_GAP;
    y = draw_separator(pixmap, theme, PADDING, y, row_w);

    let max_mb_focused = ui.settings_focus == SettingsFocus::MaxImageSizeMb;
    let (max_mb_rect, _) = draw_input_setting(
        ctx,
        pixmap,
        theme,
        "Max image size (MB)",
        "Clipboard images larger than this are not captured",
        &ui.settings_edit_max_image_mb,
        PADDING,
        y,
        row_w,
        max_mb_focused,
        if max_mb_focused { ui.settings_caret } else { 0 },
        if max_mb_focused {
            ui.settings_sel_anchor
        } else {
            0
        },
    );
    rects.max_image_mb = Some(max_mb_rect);
    y += INPUT_BLOCK_H + ROW_GAP;
    y = draw_separator(pixmap, theme, PADDING, y, row_w);

    rects.autostart = Some(draw_toggle_setting(
        ctx,
        pixmap,
        theme,
        "Start with Windows",
        None,
        PADDING,
        y,
        row_w,
        config.autostart,
    ));
    y += TOGGLE_ROW_H + ROW_GAP;
    y = draw_separator(pixmap, theme, PADDING, y, row_w);

    rects.github = Some(draw_about_section(ctx, pixmap, theme, PADDING, y, row_w));

    ui.settings_rects = rects;

    if let Some(err) = ui.settings_error.as_deref() {
        ctx.cache.draw(
            pixmap,
            err,
            PADDING,
            height - FOOTER_H - 20.0,
            12.0,
            rgba_to_color([0xDC, 0x26, 0x26, 0xFF]),
        );
    }

    ctx.cache.draw(
        pixmap,
        "Esc or \u{2190} to return",
        PADDING,
        height - 14.0,
        12.0,
        rgba_to_color(theme.text_secondary),
    );
}

fn draw_back_button(ctx: &mut UiContext<'_>, pixmap: &mut Pixmap, rect: WidgetRect, theme: &Theme) {
    const ICON: &str = "\u{2190}";
    const SIZE: f32 = 18.0;
    let glyph_w = ctx.cache.measure(ICON, SIZE);
    ctx.cache.draw(
        pixmap,
        ICON,
        rect.x + (rect.w - glyph_w) / 2.0,
        rect.y + rect.h * 0.72,
        SIZE,
        rgba_to_color(theme.text_primary),
    );
}

fn draw_about_section(
    ctx: &mut UiContext<'_>,
    pixmap: &mut Pixmap,
    theme: &Theme,
    x: f32,
    y: f32,
    width: f32,
) -> WidgetRect {
    let link_rect = WidgetRect {
        x,
        y: y + 36.0,
        w: width,
        h: 20.0,
    };
    let hovered = link_rect.contains(ctx.mouse_x, ctx.mouse_y);

    ctx.cache.draw(
        pixmap,
        "About",
        x,
        y + 4.0,
        13.0,
        rgba_to_color(theme.text_primary),
    );
    let version_line = format!("Version {APP_VERSION}");
    ctx.cache.draw(
        pixmap,
        &version_line,
        x,
        y + 20.0,
        12.0,
        rgba_to_color(theme.text_secondary),
    );
    ctx.cache.draw(
        pixmap,
        "Source code, issues, and development progress on GitHub",
        x,
        y + 58.0,
        11.0,
        rgba_to_color(theme.text_secondary),
    );

    if hovered {
        fill_rect(
            pixmap,
            link_rect.x,
            link_rect.y,
            link_rect.w,
            link_rect.h,
            rgba_to_color(theme.selection),
        );
        ctx.hot_widget = ctx.hot_widget.saturating_add(1);
    }
    ctx.cache.draw(
        pixmap,
        GITHUB_LINK_LABEL,
        x,
        link_rect.y + 14.0,
        12.0,
        rgba_to_color(theme.accent),
    );

    link_rect
}

fn draw_separator(pixmap: &mut Pixmap, theme: &Theme, x: f32, y: f32, width: f32) -> f32 {
    fill_rect(
        pixmap,
        x,
        y + SEPARATOR_TOP,
        width,
        1.0,
        rgba_to_color(theme.divider),
    );
    y + SEPARATOR_H
}

fn draw_field_caret(
    pixmap: &mut Pixmap,
    cache: &mut GlyphCache,
    value: &str,
    caret: usize,
    rect: WidgetRect,
    scroll_x: f32,
    theme: &Theme,
) {
    let caret_x = search_caret_pixel_x(cache, value, rect, caret, scroll_x);
    let top = rect.y + 5.0;
    let h = rect.h - 10.0;
    fill_rect(pixmap, caret_x, top, 1.5, h, rgba_to_color(theme.accent));
}

#[allow(clippy::too_many_arguments)]
fn draw_toggle_setting<'a>(
    ctx: &mut UiContext<'a>,
    pixmap: &mut Pixmap,
    theme: &Theme,
    label: &str,
    description: Option<&str>,
    x: f32,
    y: f32,
    width: f32,
    enabled: bool,
) -> WidgetRect {
    let rect = WidgetRect {
        x,
        y,
        w: width,
        h: TOGGLE_ROW_H,
    };

    let toggle_w = 44.0;
    let toggle_h = 22.0;
    let toggle_x = x + width - toggle_w;
    let toggle_y = y + (TOGGLE_ROW_H - toggle_h) / 2.0;

    // Align the main label optically with the toggle for every row.
    // Toggle optical centre = toggle_y + toggle_h/2 = y + 14 + 11 = y + 25.
    // For 14 px Roboto, cap-height ≈ 10 px → optical centre ≈ baseline − 5 px.
    // Setting text centre == toggle centre:  baseline = y + 25 + 5 = y + 30.
    let label_baseline = toggle_y + toggle_h / 2.0 + 5.0;

    ctx.cache.draw(
        pixmap,
        label,
        x,
        label_baseline,
        14.0,
        rgba_to_color(theme.text_primary),
    );
    if let Some(desc) = description {
        // Description sits 15 px below the label baseline (just under the
        // toggle bottom at y + 36) and still within the 50 px row height.
        ctx.cache.draw(
            pixmap,
            desc,
            x,
            label_baseline + 15.0,
            11.0,
            rgba_to_color(theme.text_secondary),
        );
    }
    draw_toggle(
        pixmap,
        toggle_x,
        toggle_y,
        toggle_w,
        toggle_h,
        enabled,
        theme.accent,
        theme.divider,
    );

    if rect.contains(ctx.mouse_x, ctx.mouse_y) {
        ctx.hot_widget = ctx.hot_widget.saturating_add(1);
    }

    rect
}

#[allow(clippy::too_many_arguments)]
fn draw_input_setting<'a>(
    ctx: &mut UiContext<'a>,
    pixmap: &mut Pixmap,
    theme: &Theme,
    label: &str,
    hint: &str,
    value: &str,
    x: f32,
    y: f32,
    width: f32,
    focused: bool,
    caret: usize,
    sel_anchor: usize,
) -> (WidgetRect, bool) {
    ctx.cache.draw(
        pixmap,
        label,
        x,
        y + 4.0,
        13.0,
        rgba_to_color(theme.text_primary),
    );
    ctx.cache.draw(
        pixmap,
        hint,
        x,
        y + 20.0,
        11.0,
        rgba_to_color(theme.text_secondary),
    );

    let input_y = y + 38.0;
    let input_rect = WidgetRect {
        x,
        y: input_y,
        w: width,
        h: INPUT_H,
    };
    let inner_w = search_text_inner_width(input_rect);
    let scroll_x = search_scroll_x(ctx.cache, value, inner_w, caret);
    let selection = if focused && has_selection(sel_anchor, caret) {
        Some(selection_range(sel_anchor, caret))
    } else {
        None
    };
    let input = input_box(
        ctx,
        pixmap,
        value,
        "",
        x,
        input_y,
        width,
        INPUT_H,
        focused,
        SEARCH_TEXT_SIZE_PX,
        scroll_x,
        selection,
        0.0,
    );
    if focused {
        draw_field_caret(pixmap, ctx.cache, value, caret, input.rect, scroll_x, theme);
    }
    (input.rect, input.focused)
}

#[allow(dead_code)]
pub fn sync_settings_edits_from_config(ui: &mut UiState, config: &Config) {
    ui.settings_edit_max_entries = config.max_entries.to_string();
    ui.settings_edit_hotkey = config.hotkey.clone();
    ui.settings_edit_max_image_mb = format_max_image_edit(config.max_image_size_mb);
}
