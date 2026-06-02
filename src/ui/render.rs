//! Rasterize the TrayVault UI into a BGRA buffer for GDI presentation.

use crate::ui::pixmap::Pixmap;

use crate::app::App;
use crate::ui::history::{
    draw_history_list, entry_inner_width, refresh_list_layout, total_content_height, EntryLayout,
};
use crate::ui::preview::{draw_help_overlay, draw_image_preview};
use crate::ui::scroll_bar::{self, tick_now};
use crate::ui::search::{
    content_viewport_height, draw_sticky_footer, draw_sticky_header, header_total_height,
};
use crate::ui::settings::draw_settings;
use crate::ui::theme::resolve_theme;
use crate::ui::titlebar;
use crate::ui::widgets::{
    draw_context_menu, fill_rect, rgba_to_color, write_rgba_to_bgra, UiContext, PADDING,
};
use crate::ui::UiState;

/// Render the current app state into a top-down BGRA buffer for [`crate::win32::gdi::GdiBuffer`].
pub fn render_app(app: &App, ui: &mut UiState, size: (u32, u32), bgra_dst: &mut [u8]) {
    let width = size.0.max(1);
    let height = size.1.max(1);
    let theme = resolve_theme(app.config.theme);

    let Some(mut pixmap) = ui.take_scratch(width, height) else {
        return;
    };

    fill_rect(
        &mut pixmap,
        0.0,
        0.0,
        width as f32,
        height as f32,
        rgba_to_color(theme.background),
    );

    ui.begin_frame();

    let mut glyph_cache = std::mem::take(&mut ui.glyph_cache);
    let mut ctx = UiContext::new(
        ui.mouse_x,
        ui.mouse_y,
        ui.mouse_left_down,
        ui.mouse_left_pressed,
        ui.mouse_right_down,
        &theme,
        &mut glyph_cache,
        ui.active_widget,
    );

    if ui.show_settings {
        draw_settings(
            &mut pixmap,
            &theme,
            &app.config,
            app.pause_capture,
            ui,
            &mut ctx,
            titlebar::TITLE_BAR_HEIGHT,
        );
        crate::ui::titlebar::draw_title_bar(&mut pixmap, &theme, app, ui, &mut ctx, width as f32);
        ui.active_widget = ctx.active_widget;
        ui.glyph_cache = glyph_cache;
        write_rgba_to_bgra(pixmap.data(), bgra_dst);
        ui.return_scratch(pixmap);
        return;
    }

    let thumb_max_w = entry_inner_width(width as f32);
    let old_bucket = ui.thumb_cache.width_bucket();
    ui.thumb_cache.set_width_bucket(thumb_max_w);
    if ui.thumb_cache.width_bucket() != old_bucket {
        ui.thumb_load_state.reset_on_width_change();
    }
    refresh_list_layout(app, ui, thumb_max_w);
    let content_height = total_content_height(&ui.cached_list_layout);
    ui.last_content_height = content_height;

    let content_top = header_total_height();
    let content_h = content_viewport_height(height as f32);
    let content_w = (width as f32 - PADDING * 2.0).max(0.0);

    ui.scroll_offset = ui
        .scroll_offset
        .clamp(0.0, (content_height - content_h).max(0.0));

    if ui.display_indices.is_empty() {
        draw_empty_state(&mut ctx, &mut pixmap, app, content_top);
    } else {
        let (visible_count, right_click) = draw_history_list(
            &mut pixmap,
            &mut ctx,
            app,
            app.thumb_loader(),
            &mut ui.thumb_load_state,
            &mut ui.thumb_cache,
            &ui.cached_list_layout,
            ui.scroll_offset,
            content_top,
            content_h,
            PADDING,
            content_w,
            ui.mouse_x,
            ui.mouse_y,
            ui.mouse_right_down,
            &ui.expanded_text_entries,
            &mut ui.expand_button_rects,
        );
        ui.last_visible_count = visible_count;

        if let Some((entry_id, x, y)) = right_click {
            ui.context_menu = Some(crate::ui::ContextMenu { entry_id, x, y });
        }
    }

    if let Some(bar) = scroll_bar::layout_from_heights(
        width as f32,
        content_top,
        content_h,
        content_height,
        ui.scroll_offset,
    ) {
        let opacity = ui.scrollbar_opacity(tick_now());
        if opacity > 0.0 || ui.scrollbar_dragging() {
            let draw_opacity = if ui.scrollbar_dragging() {
                1.0
            } else {
                opacity
            };
            scroll_bar::draw_thumb(&mut pixmap, &theme, &bar, draw_opacity);
        }
    }

    draw_sticky_header(&mut pixmap, &theme, app, ui, &mut ctx, width as f32);
    draw_sticky_footer(
        &mut pixmap,
        &theme,
        ui,
        &mut ctx,
        width as f32,
        height as f32,
    );

    if let Some(menu) = ui.context_menu.clone() {
        let entry = app.entries.iter().find(|e| e.id == menu.entry_id);
        let items = crate::ui::widgets::context_menu_labels(entry.is_some_and(|e| e.is_pinned));
        draw_context_menu(&mut ctx, &mut pixmap, menu.x, menu.y, &items);
    }

    ui.hot_widget = ctx.hot_widget;
    ui.active_widget = ctx.active_widget;

    if let Some(entry_id) = ui.preview_entry_id {
        draw_image_preview(
            &mut pixmap,
            ctx.cache,
            &mut ui.preview_cache,
            &theme,
            app,
            app.store(),
            entry_id,
        );
    }

    if ui.show_help {
        draw_help_overlay(&mut pixmap, ctx.cache, &theme);
    }

    ui.glyph_cache = glyph_cache;
    write_rgba_to_bgra(pixmap.data(), bgra_dst);
    ui.return_scratch(pixmap);
}

fn draw_empty_state(ctx: &mut UiContext<'_>, pixmap: &mut Pixmap, app: &App, content_top: f32) {
    let msg = if app.filter_query.is_empty() && app.entries.is_empty() {
        "No clipboard history yet"
    } else {
        "No matching entries"
    };
    let sub = if app.filter_query.is_empty() && app.entries.is_empty() {
        "Copy something and it will appear here"
    } else {
        "Try a different search or filter"
    };

    let width = pixmap.width() as f32;
    let viewport_h = content_viewport_height(pixmap.height() as f32);
    // Position the text block in the vertical centre of the content area.
    let center_y = content_top + viewport_h / 2.0;

    let msg_w = ctx.cache.measure(msg, 16.0);
    let sub_w = ctx.cache.measure(sub, 13.0);
    let color = rgba_to_color(ctx.theme.text_secondary);

    ctx.cache.draw(
        pixmap,
        msg,
        (width - msg_w) / 2.0,
        center_y - 10.0,
        16.0,
        color,
    );
    ctx.cache.draw(
        pixmap,
        sub,
        (width - sub_w) / 2.0,
        center_y + 14.0,
        13.0,
        color,
    );
}

/// Exposed for scroll clamping after keyboard navigation.
#[allow(dead_code)]
pub fn list_layout_for(app: &App, ui: &mut UiState, client_width: f32) -> (Vec<EntryLayout>, f32) {
    let thumb_max_w = entry_inner_width(client_width);
    refresh_list_layout(app, ui, thumb_max_w);
    let layouts = ui.cached_list_layout.clone();
    let h = total_content_height(&layouts);
    (layouts, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ThemeMode};
    use crate::hash::hash_text;
    use crate::models::{ClipEntry, EntryKind};
    use crate::store::{LoadResult, Store};
    use std::fs;

    fn temp_data_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("trayvault-render-{prefix}-{}", std::process::id()))
    }

    #[test]
    fn render_app_produces_bgra_buffer() {
        let dir = temp_data_dir("bgra");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let config = Config {
            theme: ThemeMode::Dark,
            ..Config::default()
        };
        let store = Store::open_for_test(dir.clone());
        let mut app = App::new(config, LoadResult::default(), store);
        let mut ui = UiState::default();

        let hash = hash_text("hello");
        app.entries.push(ClipEntry {
            id: 1,
            created_at: ClipEntry::now_millis(),
            kind: EntryKind::Text,
            text: Some("hello world".into()),
            html: None,
            image: None,
            image_pixels: None,
            source_app: None,
            is_pinned: false,
            hash,
        });

        let mut bgra = vec![0u8; 320 * 240 * 4];
        render_app(&app, &mut ui, (320, 240), &mut bgra);
        assert_eq!(bgra.len(), 320 * 240 * 4);
        assert!(bgra[3] > 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn render_app_reuses_scratch_buffer_at_same_size() {
        let dir = temp_data_dir("scratch");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");

        let store = Store::open_for_test(dir.clone());
        let app = App::new(Config::default(), LoadResult::default(), store);
        let mut ui = UiState::default();
        let mut bgra = vec![0u8; 64 * 48 * 4];

        render_app(&app, &mut ui, (64, 48), &mut bgra);
        let ptr1 = ui.scratch.as_ref().expect("scratch").data().as_ptr();

        render_app(&app, &mut ui, (64, 48), &mut bgra);
        let ptr2 = ui.scratch.as_ref().expect("scratch").data().as_ptr();
        assert_eq!(ptr1, ptr2, "same dimensions should reuse pixel storage");

        let _ = fs::remove_dir_all(&dir);
    }
}
