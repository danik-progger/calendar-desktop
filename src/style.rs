use gpui::{prelude::*, *};

pub(super) const SURFACE: u32 = 0xffffff;
pub(super) const TEXT_ON_SOLID: u32 = SURFACE;
pub(super) const CLEAR_SURFACE: u32 = SURFACE << 8;
pub(super) const MODAL_OVERLAY: u32 = 0x3036404d;
pub(super) const PLACEHOLDER: u32 = 0xa0a5ae;
pub(super) const OUTSIDE_MONTH: u32 = 0xaeb3bb;
pub(super) const HOUR_LABEL_COLOR: u32 = 0x5f636e;
pub(super) const BIRTHDAYS_COLOR: u32 = 0xf2ad3d;
pub(super) const PERSONAL_COLOR: u32 = 0xe86bb5;
pub(super) const WORK_COLOR: u32 = 0x7283db;
pub(super) const HEALTH_COLOR: u32 = 0x42beb9;
pub(super) const TEXT_STRONG: u32 = 0x303744;
pub(super) const TEXT_SECONDARY: u32 = 0x626b7b;
pub(super) const MUTED: u32 = 0x737780;
pub(super) const LINE: u32 = 0xf4f4f5;
pub(super) const ACCENT: u32 = 0xff6b50;
pub(super) const CONTROL_FOCUS: u32 = TEXT_STRONG;
pub(super) const ERROR_COLOR: u32 = 0xa83926;
pub(super) const HOVER: u32 = 0xf5f5f5;
pub(super) const EVENT_SHADOW: u32 = 0x3b342920;

pub(super) fn focus_outline(style: StyleRefinement) -> StyleRefinement {
    style.border_2().border_color(rgb(CONTROL_FOCUS))
}

pub(super) fn control(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    keyboard_focus_visible: bool,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    bare_control(id, label, keyboard_focus_visible, click)
        .hover(|el| el.bg(rgb(HOVER)).text_color(rgb(TEXT_STRONG)))
}

pub(super) fn action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    keyboard_focus_visible: bool,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    bare_control(id, label, keyboard_focus_visible, click)
        .h(px(30.))
        .px(px(12.))
        .text_size(px(13.))
        .line_height(px(18.))
        .text_color(rgb(TEXT_SECONDARY))
        .font_weight(FontWeight::SEMIBOLD)
}

pub(super) fn bare_control(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    keyboard_focus_visible: bool,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .tab_index(0)
        .h(px(26.))
        .px(px(8.))
        .rounded(px(6.))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(rgb(MUTED))
        .when(keyboard_focus_visible, |el| el.focus(focus_outline))
        .on_click(click)
        .children((!label.is_empty()).then_some(label))
}
