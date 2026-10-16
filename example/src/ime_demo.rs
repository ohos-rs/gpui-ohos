use std::ops::Range;

use gpui_ohos::{
    Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, IntoElement, Pixels,
    Point, Render, TextInputAction, TextInputConfiguration, UTF16Selection, Window, canvas, div,
    prelude::*, px, rgb,
};

const ACTIONS: [TextInputAction; 8] = [
    TextInputAction::Search,
    TextInputAction::Send,
    TextInputAction::Next,
    TextInputAction::Previous,
    TextInputAction::Done,
    TextInputAction::Go,
    TextInputAction::Enter,
    TextInputAction::Unspecified,
];

/// A small real input region for exercising the platform IME configuration.
pub(super) struct ImeDemo {
    focus: FocusHandle,
    text: String,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    action_index: usize,
    confirms: usize,
}

impl ImeDemo {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            text: String::new(),
            selection: 0..0,
            marked: None,
            action_index: 0,
            confirms: 0,
        }
    }

    fn range_to_utf8(&self, range: Range<usize>) -> Range<usize> {
        let start = utf8_offset(&self.text, range.start);
        start..utf8_offset(&self.text, range.end).max(start)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.text[..range.start].encode_utf16().count()
            ..self.text[..range.end].encode_utf16().count()
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> Range<usize> {
        let range = range
            .map(|range| self.range_to_utf8(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        self.text.replace_range(range.clone(), text);
        let inserted = range.start..range.start + text.len();
        self.selection = inserted.end..inserted.end;
        inserted
    }
}

fn utf8_offset(text: &str, offset_utf16: usize) -> usize {
    let mut consumed = 0;
    for (index, character) in text.char_indices() {
        if consumed + character.len_utf16() > offset_utf16 {
            return index;
        }
        consumed += character.len_utf16();
    }
    text.len()
}

impl Render for ImeDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let focus = self.focus.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(format!(
                "IME action: {:?}; confirms: {}",
                ACTIONS[self.action_index], self.confirms
            ))
            .child(
                div()
                    .id("ime-action")
                    .p_2()
                    .bg(rgb(0x64748b))
                    .child("Cycle IME confirm key")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.action_index = (this.action_index + 1) % ACTIONS.len();
                        window.focus(&this.focus, cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("ime-input")
                    .relative()
                    .track_focus(&self.focus)
                    .h(px(64.))
                    .w_full()
                    .p_2()
                    .border_1()
                    .border_color(rgb(0x94a3b8))
                    .child(if self.text.is_empty() {
                        "Tap to type with the system IME".to_owned()
                    } else {
                        format!(
                            "{}▏{}",
                            &self.text[..self.selection.end],
                            &self.text[self.selection.end..]
                        )
                    })
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| {
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, view),
                                    cx,
                                );
                            },
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.focus(&this.focus, cx);
                    }))
                    .on_key_down(cx.listener(
                        |this, event: &gpui_ohos::KeyDownEvent, window, cx| {
                            log::info!(
                                "GPUI key: key={} char={:?} modifiers={:?} caps={} held={}",
                                event.keystroke.key,
                                event.keystroke.key_char,
                                event.keystroke.modifiers,
                                window.capslock().on,
                                event.is_held
                            );
                            match event.keystroke.key.as_str() {
                                "enter" => {
                                    this.confirms += 1;
                                    log::info!(
                                        "GPUI IME confirm: action={:?} count={}",
                                        ACTIONS[this.action_index],
                                        this.confirms
                                    );
                                }
                                "backspace" => {
                                    if this.selection.is_empty() {
                                        let cursor = this.selection.start;
                                        let previous = this.text[..cursor]
                                            .char_indices()
                                            .next_back()
                                            .map(|(index, _)| index)
                                            .unwrap_or(0);
                                        this.selection = previous..cursor;
                                    }
                                    this.replace(None, "");
                                    this.marked = None;
                                }
                                "delete" => {
                                    if this.selection.is_empty() {
                                        let cursor = this.selection.end;
                                        let end = this.text[cursor..]
                                            .chars()
                                            .next()
                                            .map_or(cursor, |character| {
                                                cursor + character.len_utf8()
                                            });
                                        this.selection = cursor..end;
                                    }
                                    this.replace(None, "");
                                    this.marked = None;
                                }
                                "left" | "right" | "home" | "end" => {
                                    let cursor = match event.keystroke.key.as_str() {
                                        "left" => this.text[..this.selection.start]
                                            .char_indices()
                                            .next_back()
                                            .map_or(0, |(index, _)| index),
                                        "right" => this.text[this.selection.end..]
                                            .chars()
                                            .next()
                                            .map_or(this.selection.end, |character| {
                                                this.selection.end + character.len_utf8()
                                            }),
                                        "home" => 0,
                                        _ => this.text.len(),
                                    };
                                    this.selection = cursor..cursor;
                                    this.marked = None;
                                }
                                _ => return,
                            }
                            log::info!(
                                "GPUI IME edit: units={} selection={:?}",
                                this.text.encode_utf16().count(),
                                this.range_to_utf16(&this.selection)
                            );
                            cx.stop_propagation();
                            cx.notify();
                        },
                    )),
            )
    }
}

impl EntityInputHandler for ImeDemo {
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = self.range_to_utf8(range);
        self.marked = None;
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.text.encode_utf16().count())
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_to_utf8(range);
        *adjusted = Some(self.range_to_utf16(&range));
        Some(self.text[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(range, text);
        self.marked = None;
        log::info!("GPUI IME text length: {}", self.text.encode_utf16().count());
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inserted = self.replace(range, text);
        if let Some(selection) = selection {
            let start = utf8_offset(text, selection.start);
            let end = utf8_offset(text, selection.end).max(start);
            self.selection = inserted.start + start..inserted.start + end;
        }
        self.marked = (!text.is_empty()).then_some(inserted);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.text.encode_utf16().count())
    }

    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> TextInputConfiguration {
        TextInputConfiguration {
            input_action: ACTIONS[self.action_index],
            ..Default::default()
        }
    }
}
